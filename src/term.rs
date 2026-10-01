use std::io;
use std::mem::MaybeUninit;

use portable_pty::PtySize;

use crate::error::Result;

pub struct RawMode {
    fd: i32,
    original: libc::termios,
    active: bool,
}

impl RawMode {
    pub fn enter(fd: i32) -> Result<Self> {
        let original = get_termios(fd)?;
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        set_termios(fd, &raw)?;
        Ok(Self {
            fd,
            original,
            active: true,
        })
    }

    pub fn suspend(&mut self) -> Result<()> {
        if self.active {
            set_termios(self.fd, &self.original)?;
            self.active = false;
        }
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        if !self.active {
            let mut raw = self.original;
            unsafe { libc::cfmakeraw(&mut raw) };
            set_termios(self.fd, &raw)?;
            self.active = true;
        }
        Ok(())
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.active {
            let _ = set_termios(self.fd, &self.original);
        }
    }
}

pub fn terminal_size(fd: i32) -> PtySize {
    let mut winsz = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsz) };
    if rc == -1 || winsz.ws_row == 0 || winsz.ws_col == 0 {
        return PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
    }
    PtySize {
        rows: winsz.ws_row,
        cols: winsz.ws_col,
        pixel_width: winsz.ws_xpixel,
        pixel_height: winsz.ws_ypixel,
    }
}

pub fn is_tty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

fn get_termios(fd: i32) -> io::Result<libc::termios> {
    unsafe {
        let mut termios = MaybeUninit::<libc::termios>::uninit();
        if libc::tcgetattr(fd, termios.as_mut_ptr()) == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(termios.assume_init())
    }
}

fn set_termios(fd: i32, termios: &libc::termios) -> io::Result<()> {
    let rc = unsafe { libc::tcsetattr(fd, libc::TCSANOW, termios) };
    if rc == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn suspend_process(child_pid: Option<u32>, raw: &mut RawMode) -> Result<()> {
    raw.suspend()?;
    if let Some(pid) = child_pid {
        unsafe { libc::kill(pid as i32, libc::SIGTSTP) };
    }
    unsafe {
        libc::signal(libc::SIGTSTP, libc::SIG_DFL);
        libc::kill(libc::getpid(), libc::SIGTSTP);
        libc::signal(libc::SIGTSTP, libc::SIG_IGN);
    }
    raw.resume()?;
    if let Some(pid) = child_pid {
        unsafe { libc::kill(pid as i32, libc::SIGCONT) };
    }
    Ok(())
}
