use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ccword::engine::{Engine, UserAction};
use ccword::session::spawn_for_test;
use portable_pty::PtySize;

fn fake_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_ccword-fake-tui"))
}

fn wait_log(path: &std::path::Path, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) || start.elapsed() > timeout {
            return text;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_banner(rx: &mpsc::Receiver<Vec<u8>>, timeout: Duration) -> Vec<u8> {
    let start = Instant::now();
    let mut out = Vec::new();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                out.extend(chunk);
                if out.windows(3).any(|w| w == "❯".as_bytes()) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    out
}

#[test]
fn pty_child_receives_typed_text_resize_and_exit_status() {
    let log = std::env::temp_dir().join(format!("ccword-fake-{}.log", std::process::id()));
    let script = std::env::temp_dir().join(format!("ccword-fake-{}.sh", std::process::id()));
    let _ = std::fs::remove_file(&log);
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexport CCWORD_FAKE_LOG={}\nexec {}\n",
            log.display(),
            fake_bin().display()
        ),
    )
    .unwrap();
    let _ = std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755));
    let mut pty = spawn_for_test(
        &script,
        &[],
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    )
    .unwrap();
    let reader = std::mem::replace(&mut pty.reader, Box::new(std::io::empty()));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0; 2048];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let banner = read_banner(&rx, Duration::from_secs(2));
    assert!(
        banner.windows(3).any(|w| w == "❯".as_bytes()),
        "missing prompt: {banner:?}"
    );
    pty.writer.write_all(b"he\r").unwrap();
    pty.writer.flush().unwrap();
    let submitted = wait_log(&log, "SUBMITTED:he", Duration::from_secs(2));
    assert!(submitted.contains("SUBMITTED:he"), "{submitted}");
    assert!(!submitted.contains("SUBMITTED:help"), "{submitted}");
    pty.master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let sized = wait_log(&log, "SIZE:30x100", Duration::from_secs(2));
    assert!(sized.contains("SIZE:30x100"), "{sized}");
    pty.writer.write_all(&[0x04]).unwrap();
    pty.writer.flush().unwrap();
    let status = pty.child.wait().unwrap();
    assert_eq!(status.exit_code(), 42);
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn engine_does_not_send_a_visible_hint_to_the_child_bytes() {
    let mut engine = Engine::new(24, 80, true);
    let ansi = format!(
        "\x1b[10;1H{rule}\x1b[11;1H❯ he\x1b[11;5H\x1b[7m \x1b[27m\x1b[12;1H{rule}",
        rule = "─".repeat(80)
    );
    let (_restore, child) = engine.on_child(ansi.as_bytes());
    assert_eq!(child, ansi.as_bytes());
    engine.remember_hint("lp");
    let drawn = engine.overlay_bytes("lp").unwrap();
    assert!(drawn.windows(2).any(|w| w == b"lp"));
    let actions = engine.on_user(b"\r");
    assert_eq!(actions, vec![UserAction::Forward(b"\r".to_vec())]);
    let actions = {
        engine.remember_hint("lp");
        engine.on_user(b"\x1b[C")
    };
    match &actions[0] {
        UserAction::Insert(bytes) => {
            assert_eq!(bytes, b"lp ");
            assert!(!bytes.windows(2).any(|w| w == b"\x1b["));
        }
        other => panic!("expected insert, got {other:?}"),
    }
}
