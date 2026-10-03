use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::apple::{AppleCompleter, WordCompleter};
use crate::complete::hint_for;
use crate::config::{Config, Mode};
use crate::engine::{Engine, UserAction};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::resolve::resolve_claude;
use crate::store::Store;
use crate::term::{self, RawMode};

static WINCH: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);

pub fn launch(args: &[std::ffi::OsString]) -> Result<i32> {
    if std::env::var_os("CCWORD_INNER").is_some() {
        let exe = std::env::current_exe()?;
        let resolved = resolve_claude(
            &exe,
            std::env::var_os("CCWORD_CLAUDE").as_deref(),
            std::env::var_os("PATH").as_deref(),
        )?;
        if resolved == exe {
            return Err(Error::RecursiveLaunch { path: exe });
        }
    }
    let exe = std::env::current_exe()?;
    let claude = resolve_claude(
        &exe,
        std::env::var_os("CCWORD_CLAUDE").as_deref(),
        std::env::var_os("PATH").as_deref(),
    )?;
    let interactive = term::is_tty(libc::STDIN_FILENO)
        && term::is_tty(libc::STDOUT_FILENO)
        && std::env::var_os("CCWORD_DISABLE").is_none();
    if !interactive {
        return exec_passthrough(&claude, args);
    }
    let paths = Paths::system()?;
    let config = Config::load(&paths);
    run_pty(&claude, args, &paths, config)
}

fn exec_passthrough(claude: &Path, args: &[std::ffi::OsString]) -> Result<i32> {
    let mut cmd = Command::new(claude);
    cmd.args(args);
    cmd.env("CCWORD_INNER", "1");
    let status = cmd.status()?;
    Ok(status_code_std(&status))
}

fn status_code_std(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    128 + libc::SIGTERM
}

pub fn run_pty(
    program: &Path,
    args: &[std::ffi::OsString],
    paths: &Paths,
    mut config: Config,
) -> Result<i32> {
    let size = term::terminal_size(libc::STDOUT_FILENO);
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(size)?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    cmd.set_controlling_tty(true);
    cmd.env("CCWORD_INNER", "1");
    cmd.env("CCWORD_WRAPPER", "1");
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let pid = child.process_id();
    let reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let master = pair.master;
    install_signals()?;

    let mut raw = RawMode::enter(libc::STDIN_FILENO)?;
    let (stdin_tx, stdin_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
    let (pty_tx, pty_rx) = crossbeam_channel::unbounded::<io::Result<Vec<u8>>>();
    thread::spawn(move || stdin_loop(stdin_tx));
    thread::spawn(move || pty_loop(reader, pty_tx));

    let apple = AppleCompleter::new();
    let mut store = if matches!(config.mode, Mode::Ngram | Mode::Auto) || config.learning {
        Store::open(paths).ok()
    } else {
        None
    };
    let mut engine = Engine::new(size.rows, size.cols, config.right_arrow_appends_space);
    let mut stdout = io::stdout();
    let mut last_reload = std::time::Instant::now();
    let code = loop {
        if STOP.load(Ordering::Relaxed) {
            if let Some(pid) = pid {
                unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            }
            break wait_child(&mut *child);
        }
        if WINCH.swap(false, Ordering::Relaxed) {
            let size = term::terminal_size(libc::STDOUT_FILENO);
            let _ = master.resize(size);
            engine.resize(size.rows, size.cols);
        }
        crossbeam_channel::select! {
            recv(pty_rx) -> msg => {
                match msg {
                    Ok(Ok(bytes)) if !bytes.is_empty() => {
                        let (restore, child_bytes) = engine.on_child(&bytes);
                        if !restore.is_empty() {
                            let _ = stdout.write_all(&restore);
                        }
                        let _ = stdout.write_all(&child_bytes);
                        let _ = stdout.flush();
                        maybe_overlay(&mut engine, &mut stdout, &config, store.as_ref(), &apple);
                    }
                    Ok(Ok(_)) | Err(_) => break wait_child(&mut *child),
                    Ok(Err(_)) => break wait_child(&mut *child),
                }
            }
            recv(stdin_rx) -> msg => {
                let Ok(bytes) = msg else { break wait_child(&mut *child) };
                let mut proxy = Proxy { writer: &mut writer, stdout: &mut stdout, raw: &mut raw, pid, master: master.as_ref() };
                handle_user(&mut engine, &bytes, &mut proxy, &config, store.as_mut())?;
            }
            default(Duration::from_millis(8)) => {
                if let Some(action) = engine.flush_escape() {
                    let mut proxy = Proxy { writer: &mut writer, stdout: &mut stdout, raw: &mut raw, pid, master: master.as_ref() };
                    apply_action(action, &mut engine, &mut proxy)?;
                }
                if engine.sync_idle() {
                    maybe_overlay(&mut engine, &mut stdout, &config, store.as_ref(), &apple);
                }
                if last_reload.elapsed() > Duration::from_secs(1) {
                    config = Config::load(paths);
                    engine_set_space(&mut engine, config.right_arrow_appends_space);
                    if (matches!(config.mode, Mode::Ngram | Mode::Auto) || config.learning) && store.is_none() {
                        store = Store::open(paths).ok();
                    }
                    last_reload = std::time::Instant::now();
                }
            }
        }
    };
    drop(raw);
    code
}

fn engine_set_space(engine: &mut Engine, append: bool) {
    engine.set_append_space(append);
}

struct Proxy<'a> {
    writer: &'a mut dyn Write,
    stdout: &'a mut dyn Write,
    raw: &'a mut RawMode,
    pid: Option<u32>,
    master: &'a dyn MasterPty,
}

fn handle_user(
    engine: &mut Engine,
    bytes: &[u8],
    proxy: &mut Proxy<'_>,
    config: &Config,
    mut store: Option<&mut Store>,
) -> Result<()> {
    let submitted = engine.view().buffer.clone();
    let learn = engine.view().confident && engine.view().at_end && !engine.view().native_menu;
    for action in engine.on_user(bytes) {
        if matches!(&action, UserAction::Forward(raw) if raw.as_slice() == b"\r" || raw.as_slice() == b"\n")
            && learn
            && config.learning
        {
            if let Some(store) = store.as_mut() {
                let _ =
                    store.learn_prompt(&submitted, crate::config::now_ms(), config.half_life_days);
            }
        }
        apply_action(action, engine, proxy)?;
    }
    Ok(())
}

fn apply_action(action: UserAction, engine: &mut Engine, proxy: &mut Proxy<'_>) -> Result<()> {
    match action {
        UserAction::Forward(bytes) | UserAction::Insert(bytes) => {
            clear_overlay(engine, proxy.stdout)?;
            proxy.writer.write_all(&bytes)?;
            proxy.writer.flush()?;
        }
        UserAction::Suspend => {
            clear_overlay(engine, proxy.stdout)?;
            term::suspend_process(proxy.pid, proxy.raw)?;
            let size = term::terminal_size(libc::STDOUT_FILENO);
            let _ = proxy.master.resize(size);
            engine.resize(size.rows, size.cols);
        }
    }
    Ok(())
}

fn clear_overlay(engine: &mut Engine, stdout: &mut dyn Write) -> Result<()> {
    let (restore, _) = engine.on_child(&[]);
    if !restore.is_empty() {
        stdout.write_all(&restore)?;
        stdout.flush()?;
    }
    Ok(())
}

fn maybe_overlay(
    engine: &mut Engine,
    stdout: &mut dyn Write,
    config: &Config,
    store: Option<&Store>,
    apple: &dyn WordCompleter,
) {
    if !engine.sync_idle() || config.mode == Mode::Off {
        return;
    }
    let Some(partial) = engine.view().partial() else {
        engine.clear_hint();
        return;
    };
    let suffix = if let Some(suffix) = engine.cached_suffix() {
        suffix.to_string()
    } else {
        let buffer = engine.view().buffer.clone();
        let Some(hint) = hint_for(config, store, apple, &buffer, &partial) else {
            engine.clear_hint();
            return;
        };
        engine.remember_hint(&hint.suffix);
        hint.suffix
    };
    if let Some(bytes) = engine.overlay_bytes(&suffix) {
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }
}

fn wait_child(child: &mut dyn portable_pty::Child) -> Result<i32> {
    let status = child.wait()?;
    if let Some(sig) = status.signal() {
        return Ok(128 + signal_number(sig));
    }
    Ok(status.exit_code() as i32)
}

fn signal_number(name: &str) -> i32 {
    match name {
        "SIGHUP" => libc::SIGHUP,
        "SIGINT" => libc::SIGINT,
        "SIGQUIT" => libc::SIGQUIT,
        "SIGILL" => libc::SIGILL,
        "SIGTRAP" => libc::SIGTRAP,
        "SIGABRT" => libc::SIGABRT,
        "SIGBUS" => libc::SIGBUS,
        "SIGFPE" => libc::SIGFPE,
        "SIGKILL" => libc::SIGKILL,
        "SIGUSR1" => libc::SIGUSR1,
        "SIGSEGV" => libc::SIGSEGV,
        "SIGUSR2" => libc::SIGUSR2,
        "SIGPIPE" => libc::SIGPIPE,
        "SIGALRM" => libc::SIGALRM,
        "SIGTERM" => libc::SIGTERM,
        _ => 1,
    }
}

fn stdin_loop(tx: Sender<Vec<u8>>) {
    let mut stdin = io::stdin();
    let mut buf = [0; 1024];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn pty_loop(mut reader: Box<dyn Read + Send>, tx: Sender<io::Result<Vec<u8>>>) {
    let mut buf = [0; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Ok(Vec::new()));
                break;
            }
            Ok(n) => {
                if tx.send(Ok(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => {
                let _ = tx.send(Err(err));
                break;
            }
        }
    }
}

fn install_signals() -> Result<()> {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_winch as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        if libc::sigaction(libc::SIGWINCH, &action, std::ptr::null_mut()) == -1 {
            return Err(io::Error::last_os_error().into());
        }
        action.sa_sigaction = on_stop as *const () as usize;
        for sig in [libc::SIGTERM, libc::SIGHUP] {
            if libc::sigaction(sig, &action, std::ptr::null_mut()) == -1 {
                return Err(io::Error::last_os_error().into());
            }
        }
        libc::signal(libc::SIGTSTP, libc::SIG_IGN);
    }
    Ok(())
}

extern "C" fn on_winch(_: libc::c_int) {
    WINCH.store(true, Ordering::Relaxed);
}

extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn spawn_for_test(
    program: &Path,
    args: &[std::ffi::OsString],
    size: PtySize,
) -> Result<TestPty> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(size)?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.set_controlling_tty(true);
    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    Ok(TestPty {
        master: pair.master,
        reader,
        writer,
        child,
    })
}

pub struct TestPty {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}
