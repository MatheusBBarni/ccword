use std::io::{self, Write};

use crate::config::{Config, Mode};
use crate::error::Result;
use crate::paths::Paths;
use crate::term::RawMode;

struct Screen {
    _raw: RawMode,
}

impl Screen {
    fn enter() -> Result<Self> {
        let raw = RawMode::enter(libc::STDIN_FILENO)?;
        io::stdout().write_all(b"\x1b[?1049h\x1b[?25l")?;
        Ok(Self { _raw: raw })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = io::stdout().write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

pub fn run(paths: &Paths) -> Result<i32> {
    let mut config = Config::load(paths);
    let mut text = [
        config.language.clone(),
        config.min_confidence.to_string(),
        config.min_support.to_string(),
        config.half_life_days.to_string(),
    ];
    let mut errors: [Option<String>; 8] = std::array::from_fn(|_| None);
    let _screen = Screen::enter()?;
    let size = crate::term::terminal_size(libc::STDOUT_FILENO);
    if size.rows < 16 || size.cols < 80 {
        let mut stdout = io::stdout().lock();
        write!(
            stdout,
            "\x1b[H\x1b[2JTerminal too small\r\nminimum 80 columns x 16 rows\r\nEscape: cancel\r\n"
        )?;
        stdout.flush()?;
        while !matches!(read_byte()?, 0x1b | 0x03) {}
        return Ok(0);
    }
    let mut stdout = io::stdout().lock();
    let mut focus = 0usize;
    loop {
        draw(&mut stdout, &config, &text, &errors, focus)?;
        let byte = read_byte()?;
        let key = if byte == 0x1b && next_byte()? {
            let seq = [read_byte()?, read_byte()?];
            match seq {
                [b'[', b'Z'] => Key::Previous,
                [b'[', b'A'] => Key::Previous,
                [b'[', b'B'] => Key::Next,
                [b'[', b'C'] => Key::Change,
                [b'[', b'D'] => Key::ChangeBack,
                _ => Key::Ignore,
            }
        } else {
            match byte {
                0x1b | 0x03 => Key::Cancel,
                b'\t' => Key::Next,
                b'\r' | b'\n' => Key::Save,
                b' ' if matches!(focus, 0 | 2 | 6 | 7) => Key::Change,
                0x15 => Key::Clear,
                0x7f | 0x08 => Key::Backspace,
                b if b.is_ascii_graphic() || b == b' ' => Key::Character(b as char),
                b if b >= 0xc2 => read_character(b)?.map_or(Key::Ignore, Key::Character),
                _ => Key::Ignore,
            }
        };
        match key {
            Key::Cancel => return Ok(0),
            Key::Next => focus = (focus + 1) % 8,
            Key::Previous => focus = (focus + 7) % 8,
            Key::Change | Key::ChangeBack => match focus {
                0 => {
                    let modes = [Mode::Off, Mode::Auto, Mode::Apple, Mode::Ngram];
                    let index = modes.iter().position(|mode| *mode == config.mode).unwrap();
                    config.mode =
                        modes[(index + if matches!(key, Key::Change) { 1 } else { 3 }) % 4];
                }
                2 => config.learning = !config.learning,
                6 => config.debug = !config.debug,
                7 => config.right_arrow_appends_space = !config.right_arrow_appends_space,
                _ => {}
            },
            Key::Clear => {
                if let Some(value) = edit_field(&mut text, focus) {
                    value.clear();
                    errors[focus] = None;
                }
            }
            Key::Backspace => {
                if let Some(value) = edit_field(&mut text, focus) {
                    value.pop();
                    errors[focus] = None;
                }
            }
            Key::Character(ch) => {
                if let Some(value) = edit_field(&mut text, focus) {
                    value.push(ch);
                    errors[focus] = None;
                }
            }
            Key::Save => {
                let mut valid = true;
                for (field, name, value) in [
                    (1, "language", &text[0]),
                    (3, "min-confidence", &text[1]),
                    (4, "min-support", &text[2]),
                    (5, "half-life-days", &text[3]),
                ] {
                    errors[field] = config.set(name, value).err().map(|err| err.to_string());
                    valid &= errors[field].is_none();
                }
                if valid {
                    config.save(paths)?;
                    return Ok(0);
                }
            }
            Key::Ignore => {}
        }
    }
}

#[derive(PartialEq)]
enum Key {
    Cancel,
    Next,
    Previous,
    Change,
    ChangeBack,
    Clear,
    Backspace,
    Character(char),
    Save,
    Ignore,
}

fn read_byte() -> Result<u8> {
    let mut byte = 0u8;
    loop {
        let n = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        if n == 1 {
            return Ok(byte);
        }
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err.into());
        }
    }
}

fn next_byte() -> Result<bool> {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut fd, 1, 30) };
    if result < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(result > 0 && fd.revents & libc::POLLIN != 0)
}

fn read_character(first: u8) -> Result<Option<char>> {
    let count = match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Ok(None),
    };
    let mut bytes = [0u8; 4];
    bytes[0] = first;
    for byte in &mut bytes[1..count] {
        *byte = read_byte()?;
    }
    Ok(std::str::from_utf8(&bytes[..count])
        .ok()
        .and_then(|text| text.chars().next()))
}
fn edit_field(text: &mut [String; 4], focus: usize) -> Option<&mut String> {
    let index = match focus {
        1 => 0,
        3 => 1,
        4 => 2,
        5 => 3,
        _ => return None,
    };
    Some(&mut text[index])
}

fn draw(
    out: &mut impl Write,
    config: &Config,
    text: &[String; 4],
    errors: &[Option<String>; 8],
    focus: usize,
) -> Result<()> {
    write!(out, "\x1b[H\x1b[2Jccword settings\r\n\r\n")?;
    let language = if text[0].is_empty() {
        "system default"
    } else {
        &text[0]
    };
    let fields: [(&str, &str); 8] = [
        ("Mode", config.mode.as_str()),
        ("Language", language),
        ("Learning", if config.learning { "on" } else { "off" }),
        ("Min confidence", &text[1]),
        ("Min support", &text[2]),
        ("Half-life days", &text[3]),
        ("Debug", if config.debug { "on" } else { "off" }),
        (
            "Right Arrow appends space",
            if config.right_arrow_appends_space {
                "on"
            } else {
                "off"
            },
        ),
    ];
    for (index, (label, value)) in fields.iter().enumerate() {
        write!(out, "{} {label}: ", if index == focus { ">" } else { " " })?;
        if index == 1 {
            for ch in value.chars() {
                if ch.is_control() {
                    write!(out, "?")?;
                } else {
                    write!(out, "{ch}")?;
                }
            }
        } else {
            write!(out, "{value}")?;
        }
        if let Some(error) = &errors[index] {
            write!(out, "  Error: {error}")?;
        }
        write!(out, "\r\n")?;
    }
    write!(
        out,
        "\r\nTab/arrows: navigate  Space: toggle  Ctrl-U: clear  Enter: save  Escape: cancel\r\n"
    )?;
    out.flush()?;
    Ok(())
}
