//! Keyboard translation from UI key names to Chromium key events.
//!
//! The UI layer reports keys by name (GPUI's `"a"`, `"enter"`, `"pageup"`,
//! ...). Chromium wants a Windows virtual-key code for `KeyboardEvent.keyCode`
//! and a platform scan code for `KeyboardEvent.code`; on Linux the latter is
//! an XKB keycode. Scan codes here assume a US layout, which is what
//! `KeyboardEvent.code` is defined against.

use cef::{KeyEvent, KeyEventType};

pub const MOD_SHIFT: u32 = 1 << 1;
pub const MOD_CONTROL: u32 = 1 << 2;
pub const MOD_ALT: u32 = 1 << 3;
pub const MOD_LEFT_BUTTON: u32 = 1 << 4;
pub const MOD_MIDDLE_BUTTON: u32 = 1 << 5;
pub const MOD_RIGHT_BUTTON: u32 = 1 << 6;
pub const MOD_COMMAND: u32 = 1 << 7;
pub const MOD_IS_REPEAT: u32 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    Down,
    Up,
}

/// One physical key transition.
#[derive(Clone, Debug)]
pub struct KeyInput {
    /// UI key name, lowercase (`"a"`, `"enter"`, `"f5"`, `"["`).
    pub key: String,
    pub phase: KeyPhase,
    /// `MOD_*` flags.
    pub modifiers: u32,
    pub repeat: bool,
}

/// `(windows virtual key code, XKB keycode, character for CHAR events)`
fn lookup(key: &str) -> Option<(i32, i32, Option<u16>)> {
    let k = key;
    let letter = |c: u8| -> Option<(i32, i32, Option<u16>)> {
        const XKB: [i32; 26] = [
            38, 56, 54, 40, 26, 41, 42, 43, 31, 44, 45, 46, 58, 57, 32, 33, 24, 27, 39, 28, 30,
            55, 25, 53, 29, 52,
        ];
        let i = (c - b'a') as usize;
        Some((0x41 + i as i32, XKB[i], None))
    };
    if k.len() == 1 {
        let c = k.as_bytes()[0];
        return match c {
            b'a'..=b'z' => letter(c),
            b'A'..=b'Z' => letter(c.to_ascii_lowercase()),
            b'1'..=b'9' => Some((0x30 + (c - b'0') as i32, 10 + (c - b'1') as i32, None)),
            b'0' => Some((0x30, 19, None)),
            b'!' => Some((0x31, 10, None)),
            b'@' => Some((0x32, 11, None)),
            b'#' => Some((0x33, 12, None)),
            b'$' => Some((0x34, 13, None)),
            b'%' => Some((0x35, 14, None)),
            b'^' => Some((0x36, 15, None)),
            b'&' => Some((0x37, 16, None)),
            b'*' => Some((0x38, 17, None)),
            b'(' => Some((0x39, 18, None)),
            b')' => Some((0x30, 19, None)),
            b'-' | b'_' => Some((0xBD, 20, None)),
            b'=' | b'+' => Some((0xBB, 21, None)),
            b'[' | b'{' => Some((0xDB, 34, None)),
            b']' | b'}' => Some((0xDD, 35, None)),
            b';' | b':' => Some((0xBA, 47, None)),
            b'\'' | b'"' => Some((0xDE, 48, None)),
            b'`' | b'~' => Some((0xC0, 49, None)),
            b'\\' | b'|' => Some((0xDC, 51, None)),
            b',' | b'<' => Some((0xBC, 59, None)),
            b'.' | b'>' => Some((0xBE, 60, None)),
            b'/' | b'?' => Some((0xBF, 61, None)),
            b' ' => Some((0x20, 65, None)),
            _ => None,
        };
    }
    Some(match k {
        "escape" => (0x1B, 9, None),
        "backspace" => (0x08, 22, None),
        "tab" => (0x09, 23, None),
        "enter" | "return" => (0x0D, 36, Some(b'\r' as u16)),
        "space" => (0x20, 65, None),
        "capslock" => (0x14, 66, None),
        "f1" => (0x70, 67, None),
        "f2" => (0x71, 68, None),
        "f3" => (0x72, 69, None),
        "f4" => (0x73, 70, None),
        "f5" => (0x74, 71, None),
        "f6" => (0x75, 72, None),
        "f7" => (0x76, 73, None),
        "f8" => (0x77, 74, None),
        "f9" => (0x78, 75, None),
        "f10" => (0x79, 76, None),
        "f11" => (0x7A, 95, None),
        "f12" => (0x7B, 96, None),
        "home" => (0x24, 110, None),
        "up" => (0x26, 111, None),
        "pageup" => (0x21, 112, None),
        "left" => (0x25, 113, None),
        "right" => (0x27, 114, None),
        "end" => (0x23, 115, None),
        "down" => (0x28, 116, None),
        "pagedown" => (0x22, 117, None),
        "insert" => (0x2D, 118, None),
        "delete" => (0x2E, 119, None),
        "menu" => (0x5D, 135, None),
        "shift" => (0x10, 50, None),
        "control" => (0x11, 37, None),
        "alt" => (0x12, 64, None),
        "platform" | "super" => (0x5B, 133, None),
        _ => return None,
    })
}

impl KeyInput {
    pub fn is_known(&self) -> bool {
        lookup(&self.key).is_some()
    }

    pub(crate) fn to_cef_events(&self) -> Vec<KeyEvent> {
        let Some((vk, native, chr)) = lookup(&self.key) else {
            return Vec::new();
        };
        let mut modifiers = self.modifiers;
        if self.repeat {
            modifiers |= MOD_IS_REPEAT;
        }
        let base = KeyEvent {
            size: std::mem::size_of::<KeyEvent>(),
            modifiers,
            windows_key_code: vk,
            native_key_code: native,
            ..Default::default()
        };
        match self.phase {
            KeyPhase::Up => vec![KeyEvent {
                type_: KeyEventType::KEYUP,
                ..base
            }],
            KeyPhase::Down => {
                let mut events = vec![KeyEvent {
                    type_: KeyEventType::RAWKEYDOWN,
                    ..base.clone()
                }];
                // Printable characters arrive separately as committed text;
                // only keys whose text the UI never sees get a CHAR here.
                if let Some(c) = chr {
                    events.push(KeyEvent {
                        type_: KeyEventType::CHAR,
                        character: c,
                        unmodified_character: c,
                        windows_key_code: c as i32,
                        ..base
                    });
                }
                events
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_vk_and_xkb() {
        assert_eq!(lookup("a"), Some((0x41, 38, None)));
        assert_eq!(lookup("z"), Some((0x5A, 52, None)));
        assert_eq!(lookup("q"), Some((0x51, 24, None)));
    }

    #[test]
    fn enter_emits_char() {
        let ev = KeyInput {
            key: "enter".into(),
            phase: KeyPhase::Down,
            modifiers: 0,
            repeat: false,
        }
        .to_cef_events();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].character, b'\r' as u16);
    }

    #[test]
    fn unknown_keys_are_dropped() {
        assert!(lookup("hyper-whatever").is_none());
    }
}
