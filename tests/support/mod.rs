use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

pub struct Terminal {
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub writer: Box<dyn Write + Send>,
    #[allow(dead_code)] // Used by the terminal-state integration suite, not every test crate.
    pub master: Box<dyn MasterPty + Send>,
    rx: Receiver<Vec<u8>>,
    screen: vt100::Parser,
}

impl Terminal {
    pub fn spawn(
        args: &[&str],
        home: &Path,
        rows: u16,
        cols: u16,
        extra: &[(&str, &Path)],
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_ccword"));
        cmd.args(args);
        cmd.env("HOME", home);
        for &(key, value) in extra {
            cmd.env(key, value);
        }
        cmd.set_controlling_tty(true);
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) if tx.send(buf[..n].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        });
        Self {
            child,
            writer,
            master: pair.master,
            rx,
            screen: vt100::Parser::new(rows, cols, 0),
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn until(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let view = self.screen.screen().contents();
            if view.contains(needle) {
                return view;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                panic!("missing {needle:?} in rendered screen: {view:?}");
            }
            let bytes = self
                .rx
                .recv_timeout(left)
                .unwrap_or_else(|err| panic!("missing {needle:?}: {err}; screen: {view:?}"));
            self.screen.process(&bytes);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
