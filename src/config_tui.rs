use std::io::{self, Write};

use crate::apple::{AppleCompleter, WordCompleter};

use crate::config::{Config, Mode};
use crate::error::Result;
use crate::paths::Paths;
use crate::term::RawMode;

// Fixed foreground/background pairs keep contrast independent of the user's theme.
const BASE: &str = "\x1b[0;38;5;255;48;5;236m";
const TITLE: &str = "\x1b[1;38;5;117m";
const ACTIVE: &str = "\x1b[1;38;5;117;48;5;238m";
const LABEL: &str = "\x1b[38;5;250m";
const VALUE: &str = "\x1b[38;5;153m";
const HELP: &str = "\x1b[38;5;222m";
const ERROR: &str = "\x1b[1;38;5;210m";

struct Screen {
    _raw: RawMode,
}

impl Screen {
    fn enter() -> Result<Self> {
        let raw = RawMode::enter(libc::STDIN_FILENO)?;
        io::stdout().write_all(b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h")?;
        Ok(Self { _raw: raw })
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = io::stdout().write_all(b"\x1b[0m\x1b[?1000l\x1b[?1006l\x1b[?25h\x1b[?1049l");
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
            "{BASE}\x1b[H\x1b[2J{TITLE}Terminal too small{BASE}\r\nminimum 80 columns x 16 rows\r\n{HELP}Escape: cancel{BASE}\r\n"
        )?;
        stdout.flush()?;
        while !matches!(read_byte()?, 0x1b | 0x03) {}
        return Ok(0);
    }
    let mut stdout = io::stdout().lock();
    let mut focus = 0usize;
    let mut picker: Option<LanguagePicker> = None;
    loop {
        if let Some(choices) = picker.as_ref() {
            draw_language_picker(&mut stdout, choices, size.rows)?;
        } else {
            draw(&mut stdout, &config, &text, &errors, focus)?;
        }
        let key = read_key(focus)?;
        if let Some(choices) = picker.as_mut() {
            match key {
                Key::Cancel => picker = None,
                Key::Save | Key::Change => {
                    text[0].clear();
                    text[0].push_str(choices.selected_value());
                    errors[1] = None;
                    picker = None;
                }
                Key::Next => choices.move_selection(1),
                Key::Previous | Key::ChangeBack => choices.move_selection(-1),
                Key::Home => choices.selected = 0,
                Key::Click(row) => {
                    if let Some(value) = choices.choice_at(row, size.rows) {
                        text[0].clear();
                        text[0].push_str(value);
                        errors[1] = None;
                        picker = None;
                    }
                }
                _ => {}
            }
            continue;
        }
        match key {
            Key::Cancel => return Ok(0),
            Key::Next => focus = (focus + 1) % 8,
            Key::Previous => focus = (focus + 7) % 8,
            Key::Click(row) => {
                if (3..=10).contains(&row) {
                    focus = row - 3;
                    if focus == 1 {
                        picker = Some(LanguagePicker::open(&text[0]));
                    }
                }
            }
            Key::Change | Key::ChangeBack => match focus {
                0 => {
                    let modes = [Mode::Off, Mode::Auto, Mode::Apple, Mode::Ngram];
                    let index = modes.iter().position(|mode| *mode == config.mode).unwrap();
                    config.mode =
                        modes[(index + if matches!(key, Key::Change) { 1 } else { 3 }) % 4];
                }
                1 => picker = Some(LanguagePicker::open(&text[0])),
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
            Key::Home | Key::Ignore => {}
        }
    }
}

#[derive(PartialEq)]
enum Key {
    Cancel,
    Next,
    Previous,
    Home,
    Change,
    ChangeBack,
    Clear,
    Backspace,
    Character(char),
    Save,
    Click(usize),
    Ignore,
}

struct LanguagePicker {
    choices: Vec<String>,
    selected: usize,
}

impl LanguagePicker {
    fn open(current: &str) -> Self {
        let mut choices = AppleCompleter::new().languages();
        choices.retain(|language| !language.is_empty());
        choices.sort_unstable();
        choices.dedup();
        choices.insert(0, String::new());
        let selected = if current.is_empty() {
            0
        } else if let Some(index) = choices.iter().position(|language| language == current) {
            index
        } else {
            choices.insert(1, current.to_string());
            1
        };
        Self { choices, selected }
    }

    fn selected_value(&self) -> &str {
        &self.choices[self.selected]
    }

    fn move_selection(&mut self, step: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(step)
            .min(self.choices.len() - 1);
    }

    fn first_visible(&self, rows: u16) -> usize {
        if self.selected == 0 {
            1
        } else {
            self.selected.saturating_sub(rows as usize - 7).max(1)
        }
    }

    fn choice_at(&self, row: usize, rows: u16) -> Option<&str> {
        if row == 3 {
            return Some("");
        }
        if !(4..rows as usize - 2).contains(&row) {
            return None;
        }
        self.choices
            .get(self.first_visible(rows) + row - 4)
            .map(String::as_str)
    }
}

fn read_key(focus: usize) -> Result<Key> {
    let byte = read_byte()?;
    if byte == 0x1b && next_byte()? {
        let seq = [read_byte()?, read_byte()?];
        return Ok(match seq {
            [b'[', b'Z'] | [b'[', b'A'] => Key::Previous,
            [b'[', b'B'] => Key::Next,
            [b'[', b'C'] => Key::Change,
            [b'[', b'D'] => Key::ChangeBack,
            [b'[', b'H'] => Key::Home,
            [b'[', b'<'] => read_mouse()?,
            [b'[', b'M'] => {
                let (button, col, row) = (read_byte()?, read_byte()?, read_byte()?);
                if button == b' ' && col > 32 && row > 32 {
                    Key::Click((row - 32) as usize)
                } else {
                    Key::Ignore
                }
            }
            _ => Key::Ignore,
        });
    }
    Ok(match byte {
        0x1b | 0x03 => Key::Cancel,
        b'\t' => Key::Next,
        b'\r' | b'\n' => Key::Save,
        b' ' if matches!(focus, 0 | 1 | 2 | 6 | 7) => Key::Change,
        0x15 => Key::Clear,
        0x7f | 0x08 => Key::Backspace,
        b if b.is_ascii_graphic() || b == b' ' => Key::Character(b as char),
        b if b >= 0xc2 => read_character(b)?.map_or(Key::Ignore, Key::Character),
        _ => Key::Ignore,
    })
}

fn read_mouse() -> Result<Key> {
    let mut bytes = [0u8; 32];
    for index in 0..bytes.len() {
        let byte = read_byte()?;
        if matches!(byte, b'M' | b'm') {
            if byte == b'm' {
                return Ok(Key::Ignore);
            }
            let Ok(event) = std::str::from_utf8(&bytes[..index]) else {
                return Ok(Key::Ignore);
            };
            let mut parts = event
                .split(';')
                .filter_map(|part| part.parse::<usize>().ok());
            return Ok(
                match (parts.next(), parts.next(), parts.next(), parts.next()) {
                    (Some(0), Some(col), Some(row), None) if col > 0 => Key::Click(row),
                    _ => Key::Ignore,
                },
            );
        }
        if !byte.is_ascii_digit() && byte != b';' {
            return Ok(Key::Ignore);
        }
        bytes[index] = byte;
    }
    Ok(Key::Ignore)
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
    write!(
        out,
        "{BASE}\x1b[H\x1b[2J{TITLE}ccword settings{BASE}\r\n\r\n"
    )?;
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
        if index == focus {
            write!(out, "{ACTIVE}> {label}: \x1b[38;5;255m")?;
        } else {
            write!(out, "{BASE}  {LABEL}{label}: {VALUE}")?;
        }
        if index == 1 {
            write_safe(out, value)?;
            write!(out, "  {HELP}[Space/Click: choose]")?;
        } else {
            write!(out, "{value}")?;
        }
        if let Some(error) = &errors[index] {
            write!(out, "  {ERROR}Error: {error}")?;
        }
        write!(out, "{BASE}\r\n")?;
    }
    write!(
        out,
        "\r\n{HELP}Tab/Up/Down: move  Space: choose/toggle  Type: edit  Enter: save{BASE}\r\n"
    )?;
    let clear = if focus == 1 {
        "Ctrl-U: system default"
    } else {
        "Ctrl-U: clear field"
    };
    write!(
        out,
        "{HELP}Click Language for list  {clear}  Escape: cancel{BASE}\r\n"
    )?;
    out.flush()?;
    Ok(())
}

fn write_safe(out: &mut impl Write, value: &str) -> Result<()> {
    for ch in value.chars() {
        if ch.is_control() {
            write!(out, "?")?;
        } else {
            write!(out, "{ch}")?;
        }
    }
    Ok(())
}

fn draw_language_picker(out: &mut impl Write, picker: &LanguagePicker, rows: u16) -> Result<()> {
    write!(
        out,
        "{BASE}\x1b[H\x1b[2J{TITLE}Choose language{BASE}\r\n\r\n"
    )?;
    if picker.selected == 0 {
        write!(out, "{ACTIVE}> system default{BASE}\r\n")?;
    } else {
        write!(out, "{BASE}  {VALUE}system default{BASE}\r\n")?;
    }
    let start = picker.first_visible(rows);
    let visible = rows as usize - 6;
    for index in start..(start + visible).min(picker.choices.len()) {
        if index == picker.selected {
            write!(out, "{ACTIVE}> ")?;
        } else {
            write!(out, "{BASE}  {VALUE}")?;
        }
        write_safe(out, &picker.choices[index])?;
        write!(out, "{BASE}\r\n")?;
    }
    write!(out, "\x1b[{};1H{HELP}", rows - 1)?;
    write!(
        out,
        "Up/Down: choose ({}/{})  Home: system  Enter/Space: use  Esc: back{BASE}\r\n",
        picker.selected + 1,
        picker.choices.len()
    )?;
    out.flush()?;
    Ok(())
}
