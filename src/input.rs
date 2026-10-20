//! Keyboard and mouse protocol encoding.
//!
//! Application shortcuts are matched separately from shell input. Everything
//! that is not an application shortcut is encoded as terminal input bytes; text
//! is copied verbatim as UTF-8 and never reversed.

use winit::event::KeyEvent;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordKey {
    Char(char),
    Tab,
    Enter,
    Escape,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Space,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    key: ChordKey,
}

impl KeyChord {
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        if parts.is_empty() {
            return None;
        }
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key = None;
        for part in parts.iter() {
            if part.is_empty() {
                return None;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                "super" | "win" | "meta" => return None,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(parse_key(part)?);
                }
            }
        }
        Some(Self {
            ctrl,
            shift,
            alt,
            key: key?,
        })
    }

    pub fn canonical(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.shift {
            out.push_str("Shift+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        out.push_str(&key_name(self.key));
        out
    }

    pub fn matches(&self, event: &KeyEvent, modifiers: ModifiersState) -> bool {
        if modifiers.control_key() != self.ctrl
            || modifiers.shift_key() != self.shift
            || modifiers.alt_key() != self.alt
        {
            return false;
        }
        if key_of(event).is_some_and(|key| chord_key_eq(self.key, key)) {
            return true;
        }
        // Application shortcuts should remain usable when the user has a
        // Persian (or any non-Latin) keyboard layout active. Match the
        // physical key as a fallback; text input still uses the logical key.
        physical_key_of(event).is_some_and(|key| chord_key_eq(self.key, key))
    }
}

fn chord_key_eq(expected: ChordKey, actual: ChordKey) -> bool {
    match (expected, actual) {
        (ChordKey::Char(a), ChordKey::Char(b)) => {
            a.eq_ignore_ascii_case(&b) || (a == '=' && b == '+')
        }
        (a, b) => a == b,
    }
}

fn physical_key_of(event: &KeyEvent) -> Option<ChordKey> {
    let PhysicalKey::Code(code) = event.physical_key else {
        return None;
    };
    Some(match code {
        KeyCode::KeyA => ChordKey::Char('a'),
        KeyCode::KeyB => ChordKey::Char('b'),
        KeyCode::KeyC => ChordKey::Char('c'),
        KeyCode::KeyD => ChordKey::Char('d'),
        KeyCode::KeyE => ChordKey::Char('e'),
        KeyCode::KeyF => ChordKey::Char('f'),
        KeyCode::KeyG => ChordKey::Char('g'),
        KeyCode::KeyH => ChordKey::Char('h'),
        KeyCode::KeyI => ChordKey::Char('i'),
        KeyCode::KeyJ => ChordKey::Char('j'),
        KeyCode::KeyK => ChordKey::Char('k'),
        KeyCode::KeyL => ChordKey::Char('l'),
        KeyCode::KeyM => ChordKey::Char('m'),
        KeyCode::KeyN => ChordKey::Char('n'),
        KeyCode::KeyO => ChordKey::Char('o'),
        KeyCode::KeyP => ChordKey::Char('p'),
        KeyCode::KeyQ => ChordKey::Char('q'),
        KeyCode::KeyR => ChordKey::Char('r'),
        KeyCode::KeyS => ChordKey::Char('s'),
        KeyCode::KeyT => ChordKey::Char('t'),
        KeyCode::KeyU => ChordKey::Char('u'),
        KeyCode::KeyV => ChordKey::Char('v'),
        KeyCode::KeyW => ChordKey::Char('w'),
        KeyCode::KeyX => ChordKey::Char('x'),
        KeyCode::KeyY => ChordKey::Char('y'),
        KeyCode::KeyZ => ChordKey::Char('z'),
        KeyCode::Digit0 => ChordKey::Char('0'),
        KeyCode::Digit1 => ChordKey::Char('1'),
        KeyCode::Digit2 => ChordKey::Char('2'),
        KeyCode::Digit3 => ChordKey::Char('3'),
        KeyCode::Digit4 => ChordKey::Char('4'),
        KeyCode::Digit5 => ChordKey::Char('5'),
        KeyCode::Digit6 => ChordKey::Char('6'),
        KeyCode::Digit7 => ChordKey::Char('7'),
        KeyCode::Digit8 => ChordKey::Char('8'),
        KeyCode::Digit9 => ChordKey::Char('9'),
        KeyCode::Comma => ChordKey::Char(','),
        KeyCode::Period => ChordKey::Char('.'),
        KeyCode::Equal => ChordKey::Char('='),
        KeyCode::Minus => ChordKey::Char('-'),
        KeyCode::Slash => ChordKey::Char('/'),
        KeyCode::Backslash => ChordKey::Char('\\'),
        KeyCode::Semicolon => ChordKey::Char(';'),
        KeyCode::Quote => ChordKey::Char('\''),
        KeyCode::Backquote => ChordKey::Char('`'),
        KeyCode::Tab => ChordKey::Tab,
        KeyCode::Enter => ChordKey::Enter,
        KeyCode::Escape => ChordKey::Escape,
        KeyCode::Backspace => ChordKey::Backspace,
        KeyCode::Delete => ChordKey::Delete,
        KeyCode::Insert => ChordKey::Insert,
        KeyCode::Home => ChordKey::Home,
        KeyCode::End => ChordKey::End,
        KeyCode::PageUp => ChordKey::PageUp,
        KeyCode::PageDown => ChordKey::PageDown,
        KeyCode::ArrowUp => ChordKey::Up,
        KeyCode::ArrowDown => ChordKey::Down,
        KeyCode::ArrowLeft => ChordKey::Left,
        KeyCode::ArrowRight => ChordKey::Right,
        KeyCode::Space => ChordKey::Space,
        KeyCode::F1 => ChordKey::F(1),
        KeyCode::F2 => ChordKey::F(2),
        KeyCode::F3 => ChordKey::F(3),
        KeyCode::F4 => ChordKey::F(4),
        KeyCode::F5 => ChordKey::F(5),
        KeyCode::F6 => ChordKey::F(6),
        KeyCode::F7 => ChordKey::F(7),
        KeyCode::F8 => ChordKey::F(8),
        KeyCode::F9 => ChordKey::F(9),
        KeyCode::F10 => ChordKey::F(10),
        KeyCode::F11 => ChordKey::F(11),
        KeyCode::F12 => ChordKey::F(12),
        _ => return None,
    })
}

fn parse_key(part: &str) -> Option<ChordKey> {
    let lower = part.to_ascii_lowercase();
    Some(match lower.as_str() {
        "tab" => ChordKey::Tab,
        "enter" | "return" => ChordKey::Enter,
        "escape" | "esc" => ChordKey::Escape,
        "backspace" => ChordKey::Backspace,
        "delete" | "del" => ChordKey::Delete,
        "insert" | "ins" => ChordKey::Insert,
        "home" => ChordKey::Home,
        "end" => ChordKey::End,
        "pageup" | "pgup" => ChordKey::PageUp,
        "pagedown" | "pgdn" => ChordKey::PageDown,
        "up" => ChordKey::Up,
        "down" => ChordKey::Down,
        "left" => ChordKey::Left,
        "right" => ChordKey::Right,
        "space" => ChordKey::Space,
        "comma" => ChordKey::Char(','),
        "period" => ChordKey::Char('.'),
        "equal" | "equals" | "plus" => ChordKey::Char('='),
        "minus" | "dash" => ChordKey::Char('-'),
        "slash" => ChordKey::Char('/'),
        "backslash" => ChordKey::Char('\\'),
        "semicolon" => ChordKey::Char(';'),
        "quote" | "apostrophe" => ChordKey::Char('\''),
        "backtick" | "grave" => ChordKey::Char('`'),
        k if k.starts_with('f') && k.len() <= 3 => {
            let n: u8 = k[1..].parse().ok()?;
            if (1..=24).contains(&n) {
                ChordKey::F(n)
            } else {
                return None;
            }
        }
        _ => {
            let mut chars = part.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ChordKey::Char(ch)
        }
    })
}

fn key_name(key: ChordKey) -> String {
    match key {
        ChordKey::Char(' ') => "Space".to_owned(),
        ChordKey::Char(',') => "Comma".to_owned(),
        ChordKey::Char('=') => "Equal".to_owned(),
        ChordKey::Char('-') => "Minus".to_owned(),
        ChordKey::Char('\t') => "Tab".to_owned(),
        ChordKey::Char(ch) => ch.to_uppercase().to_string(),
        ChordKey::Tab => "Tab".to_owned(),
        ChordKey::Enter => "Enter".to_owned(),
        ChordKey::Escape => "Escape".to_owned(),
        ChordKey::Backspace => "Backspace".to_owned(),
        ChordKey::Delete => "Delete".to_owned(),
        ChordKey::Insert => "Insert".to_owned(),
        ChordKey::Home => "Home".to_owned(),
        ChordKey::End => "End".to_owned(),
        ChordKey::PageUp => "PageUp".to_owned(),
        ChordKey::PageDown => "PageDown".to_owned(),
        ChordKey::Up => "Up".to_owned(),
        ChordKey::Down => "Down".to_owned(),
        ChordKey::Left => "Left".to_owned(),
        ChordKey::Right => "Right".to_owned(),
        ChordKey::Space => "Space".to_owned(),
        ChordKey::F(n) => format!("F{n}"),
    }
}

fn key_of(event: &KeyEvent) -> Option<ChordKey> {
    match &event.logical_key {
        Key::Character(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            Some(ChordKey::Char(ch))
        }
        Key::Named(named) => Some(match named {
            NamedKey::Tab => ChordKey::Tab,
            NamedKey::Enter => ChordKey::Enter,
            NamedKey::Escape => ChordKey::Escape,
            NamedKey::Backspace => ChordKey::Backspace,
            NamedKey::Delete => ChordKey::Delete,
            NamedKey::Insert => ChordKey::Insert,
            NamedKey::Home => ChordKey::Home,
            NamedKey::End => ChordKey::End,
            NamedKey::PageUp => ChordKey::PageUp,
            NamedKey::PageDown => ChordKey::PageDown,
            NamedKey::ArrowUp => ChordKey::Up,
            NamedKey::ArrowDown => ChordKey::Down,
            NamedKey::ArrowLeft => ChordKey::Left,
            NamedKey::ArrowRight => ChordKey::Right,
            NamedKey::Space => ChordKey::Space,
            NamedKey::F1 => ChordKey::F(1),
            NamedKey::F2 => ChordKey::F(2),
            NamedKey::F3 => ChordKey::F(3),
            NamedKey::F4 => ChordKey::F(4),
            NamedKey::F5 => ChordKey::F(5),
            NamedKey::F6 => ChordKey::F(6),
            NamedKey::F7 => ChordKey::F(7),
            NamedKey::F8 => ChordKey::F(8),
            NamedKey::F9 => ChordKey::F(9),
            NamedKey::F10 => ChordKey::F(10),
            NamedKey::F11 => ChordKey::F(11),
            NamedKey::F12 => ChordKey::F(12),
            _ => return None,
        }),
        _ => None,
    }
}

/// Encode a key press for the shell. Returns `None` for a key that produces no
/// bytes (modifiers, unknown keys, dead keys).
pub fn encode_key(
    event: &KeyEvent,
    modifiers: ModifiersState,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let ctrl = modifiers.control_key();
    let alt = modifiers.alt_key();
    let shift = modifiers.shift_key();

    // Named keys first: these must not be confused with their textual
    // representation (Enter's text is "\r", Tab's text is "\t").
    if let Key::Named(named) = &event.logical_key {
        let seq: Option<Vec<u8>> = match named {
            NamedKey::Enter => Some(b"\r".to_vec()),
            NamedKey::Tab => {
                if shift {
                    Some(b"\x1b[Z".to_vec())
                } else {
                    Some(b"\t".to_vec())
                }
            }
            NamedKey::Backspace => {
                if ctrl {
                    Some(vec![0x17])
                } else {
                    Some(vec![0x7f])
                }
            }
            NamedKey::Escape => Some(vec![0x1b]),
            NamedKey::Delete => Some(csi_tilde(3, modifier_param(shift, alt, ctrl))),
            NamedKey::Insert => Some(csi_tilde(2, modifier_param(shift, alt, ctrl))),
            NamedKey::PageUp => Some(csi_tilde(5, modifier_param(shift, alt, ctrl))),
            NamedKey::PageDown => Some(csi_tilde(6, modifier_param(shift, alt, ctrl))),
            NamedKey::ArrowUp => Some(cursor_key(b'A', shift, alt, ctrl, application_cursor)),
            NamedKey::ArrowDown => Some(cursor_key(b'B', shift, alt, ctrl, application_cursor)),
            NamedKey::ArrowRight => Some(cursor_key(b'C', shift, alt, ctrl, application_cursor)),
            NamedKey::ArrowLeft => Some(cursor_key(b'D', shift, alt, ctrl, application_cursor)),
            NamedKey::Home => Some(cursor_key(b'H', shift, alt, ctrl, application_cursor)),
            NamedKey::End => Some(cursor_key(b'F', shift, alt, ctrl, application_cursor)),
            NamedKey::F1 => Some(function_key(1, shift, alt, ctrl)),
            NamedKey::F2 => Some(function_key(2, shift, alt, ctrl)),
            NamedKey::F3 => Some(function_key(3, shift, alt, ctrl)),
            NamedKey::F4 => Some(function_key(4, shift, alt, ctrl)),
            NamedKey::F5 => Some(function_key(5, shift, alt, ctrl)),
            NamedKey::F6 => Some(function_key(6, shift, alt, ctrl)),
            NamedKey::F7 => Some(function_key(7, shift, alt, ctrl)),
            NamedKey::F8 => Some(function_key(8, shift, alt, ctrl)),
            NamedKey::F9 => Some(function_key(9, shift, alt, ctrl)),
            NamedKey::F10 => Some(function_key(10, shift, alt, ctrl)),
            NamedKey::F11 => Some(function_key(11, shift, alt, ctrl)),
            NamedKey::F12 => Some(function_key(12, shift, alt, ctrl)),
            NamedKey::Space => {
                if ctrl {
                    Some(vec![0])
                } else {
                    Some(b" ".to_vec())
                }
            }
            _ => None,
        };
        if let Some(mut seq) = seq {
            if alt && !matches!(named, NamedKey::Escape) {
                seq.insert(0, 0x1b);
            }
            return Some(seq);
        }
    }

    // AltGr arrives as Ctrl+Alt with a printable text payload on Windows.
    // Prefer the OS-provided text in that case.
    if ctrl && alt {
        if let Some(text) = event.text.as_ref().filter(|t| !t.is_empty()) {
            return Some(text.as_bytes().to_vec());
        }
    }

    if ctrl {
        if let Key::Character(text) = &event.logical_key {
            if let Some(byte) = control_byte(text.chars().next().unwrap_or('\0')) {
                let mut out = vec![byte];
                if alt {
                    out.insert(0, 0x1b);
                }
                return Some(out);
            }
        }
    }

    if let Some(text) = event.text.as_ref().filter(|t| !t.is_empty()) {
        let mut bytes = text.as_bytes().to_vec();
        if alt {
            bytes.insert(0, 0x1b);
        }
        return Some(bytes);
    }

    if alt {
        if let Key::Character(text) = &event.logical_key {
            let mut bytes = vec![0x1b];
            bytes.extend_from_slice(text.as_bytes());
            return Some(bytes);
        }
    }

    None
}

pub fn control_byte(ch: char) -> Option<u8> {
    let ch = ch.to_ascii_lowercase();
    match ch {
        '@' => Some(0),
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        '[' => Some(27),
        '\\' => Some(28),
        ']' => Some(29),
        '^' => Some(30),
        '_' => Some(31),
        '?' => Some(127),
        _ => None,
    }
}

fn modifier_param(shift: bool, alt: bool, ctrl: bool) -> u8 {
    1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl)
}

fn cursor_key(letter: u8, shift: bool, alt: bool, ctrl: bool, app_cursor: bool) -> Vec<u8> {
    let m = modifier_param(shift, alt, ctrl);
    if m == 1 {
        if app_cursor {
            vec![0x1b, b'O', letter]
        } else {
            vec![0x1b, b'[', letter]
        }
    } else {
        format!("\x1b[1;{m}{}", letter as char).into_bytes()
    }
}

fn csi_tilde(code: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

fn function_key(n: u8, shift: bool, alt: bool, ctrl: bool) -> Vec<u8> {
    let m = modifier_param(shift, alt, ctrl);
    let letter = match n {
        1 => Some(b'P'),
        2 => Some(b'Q'),
        3 => Some(b'R'),
        4 => Some(b'S'),
        _ => None,
    };
    if let Some(letter) = letter {
        if m == 1 {
            vec![0x1b, b'O', letter]
        } else {
            format!("\x1b[1;{m}{}", letter as char).into_bytes()
        }
    } else {
        let code = match n {
            5 => 15,
            6 => 17,
            7 => 18,
            8 => 19,
            9 => 20,
            10 => 21,
            11 => 23,
            12 => 24,
            _ => 15,
        };
        csi_tilde(code, m)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseReport {
    pub button: MouseButton,
    pub pressed: bool,
    pub motion: bool,
    pub col: usize,
    pub row: usize,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub sgr: bool,
    pub utf8: bool,
}

pub fn encode_mouse(report: MouseReport) -> Vec<u8> {
    let mut cb = match report.button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::None => 3,
    };
    if report.motion {
        cb += 32;
    }
    if report.shift {
        cb += 4;
    }
    if report.alt {
        cb += 8;
    }
    if report.ctrl {
        cb += 16;
    }
    let col = report.col + 1;
    let row = report.row + 1;
    if report.sgr {
        let final_byte = if report.pressed { 'M' } else { 'm' };
        format!("\x1b[<{cb};{col};{row}{final_byte}").into_bytes()
    } else {
        let mut out = b"\x1b[M".to_vec();
        out.push((32 + cb).min(255) as u8);
        out.push((32 + col).min(255) as u8);
        out.push((32 + row).min(255) as u8);
        if report.utf8 {
            // The original xterm UTF-8 mouse protocol is rarely implemented;
            // SGR is preferred whenever the application asks for it. For
            // compatibility, coordinates that overflow a byte are left
            // saturated rather than emitting malformed UTF-8.
            out.truncate(3);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_shortcuts() {
        let chord = KeyChord::parse("Ctrl+Shift+T").unwrap();
        assert!(chord.ctrl && chord.shift && !chord.alt);
        assert_eq!(chord.canonical(), "Ctrl+Shift+T");
        assert_eq!(
            KeyChord::parse("Ctrl+Comma").unwrap().canonical(),
            "Ctrl+Comma"
        );
        assert_eq!(
            KeyChord::parse("Ctrl+Equal").unwrap().canonical(),
            "Ctrl+Equal"
        );
        assert!(KeyChord::parse("Ctrl+Shift+").is_none());
        assert!(KeyChord::parse("Hyper+X").is_none());
    }

    #[test]
    fn control_codes_follow_ansi_conventions() {
        assert_eq!(control_byte('c'), Some(0x03));
        assert_eq!(control_byte('D'), Some(0x04));
        assert_eq!(control_byte('z'), Some(0x1a));
        assert_eq!(control_byte('['), Some(0x1b));
        assert_eq!(control_byte(' '), None);
    }

    #[test]
    fn mouse_sgr_encoding_uses_one_based_coordinates() {
        let bytes = encode_mouse(MouseReport {
            button: MouseButton::Left,
            pressed: true,
            motion: false,
            col: 0,
            row: 0,
            shift: false,
            alt: false,
            ctrl: false,
            sgr: true,
            utf8: false,
        });
        assert_eq!(bytes, b"\x1b[<0;1;1M");
        let release = encode_mouse(MouseReport {
            button: MouseButton::Left,
            pressed: false,
            motion: false,
            col: 4,
            row: 2,
            shift: false,
            alt: false,
            ctrl: false,
            sgr: true,
            utf8: false,
        });
        assert_eq!(release, b"\x1b[<0;5;3m");
    }

    #[test]
    fn x10_mouse_encoding_is_well_formed() {
        let bytes = encode_mouse(MouseReport {
            button: MouseButton::Right,
            pressed: true,
            motion: false,
            col: 1,
            row: 2,
            shift: false,
            alt: false,
            ctrl: false,
            sgr: false,
            utf8: false,
        });
        assert_eq!(bytes, b"\x1b[M\"\"#");
    }

    #[test]
    fn function_key_sequences() {
        assert_eq!(function_key(1, false, false, false), b"\x1bOP");
        assert_eq!(function_key(5, false, false, false), b"\x1b[15~");
        assert_eq!(
            function_key(1, true, false, false),
            b"\x1b[1;2P".to_vec()
        );
    }
}

