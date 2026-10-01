use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};

static WINCH: AtomicBool = AtomicBool::new(false);
fn main() {
    install_winch();
    let _raw = RawMode::enter();
    let mut input = String::new();
    let mut buf = [0; 64];
    let mut extra = [0; 8];
    draw(&input);
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    loop {
        if WINCH.swap(false, Ordering::Relaxed) {
            log_size();
        }
        match stdin.read(&mut buf) {
            Ok(0) => std::process::exit(0),
            Ok(n) => {
                for &byte in &buf[..n] {
                    match byte {
                        0x04 => std::process::exit(42),
                        0x03 => std::process::exit(0),
                        0x7f | 0x08 => {
                            input.pop();
                            draw(&input);
                        }
                        b'\r' | b'\n' => {
                            log_line(&format!("SUBMITTED:{input}"));
                            input.clear();
                            draw(&input);
                        }
                        0x1b => {
                            // Swallow a short escape so arrow keys do not enter the buffer.
                            let _ = stdin.read(&mut extra);
                        }
                        b if b.is_ascii_graphic() || b == b' ' => {
                            input.push(byte as char);
                            if input.starts_with('/') {
                                draw_menu(&input);
                            } else {
                                draw(&input);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                log_size();
            }
            Err(_) => std::process::exit(1),
        }
    }
}

fn draw(input: &str) {
    let rule = "─".repeat(40);
    let frame = format!("\x1b[1;1H\x1b[J{rule}\r\n❯ {input}\x1b[7m \x1b[0m\r\n{rule}\r\n");
    let _ = io::stdout().write_all(frame.as_bytes());
    let _ = io::stdout().flush();
}

fn draw_menu(input: &str) {
    let rule = "─".repeat(40);
    let frame = format!(
        "\x1b[1;1H\x1b[J/add-dir Add a directory\r\n/agents Manage agents\r\n{rule}\r\n❯ {input}\x1b[7m \x1b[0m\r\n{rule}\r\n"
    );
    let _ = io::stdout().write_all(frame.as_bytes());
    let _ = io::stdout().flush();
}

fn log_line(line: &str) {
    let Some(path) = std::env::var_os("CCWORD_FAKE_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn install_winch() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_winch as *const () as usize;
        let _ = libc::sigaction(libc::SIGWINCH, &action, std::ptr::null_mut());
    }
}

extern "C" fn on_winch(_: libc::c_int) {
    WINCH.store(true, Ordering::Relaxed);
}
fn log_size() {
    let mut winsz = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &mut winsz);
    }
    log_line(&format!("SIZE:{}x{}", winsz.ws_row, winsz.ws_col));
}

struct RawMode {
    fd: i32,
    original: libc::termios,
}

impl RawMode {
    fn enter() -> Self {
        let fd = libc::STDIN_FILENO;
        let mut original = unsafe { std::mem::zeroed() };
        unsafe {
            libc::tcgetattr(fd, &mut original);
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(fd, libc::TCSANOW, &raw);
        }
        Self { fd, original }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}
