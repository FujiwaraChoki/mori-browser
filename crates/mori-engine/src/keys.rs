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

/// The modifier keys a shortcut is made of (no mouse buttons or lock states).
pub const MOD_KEYS: u32 = MOD_SHIFT | MOD_CONTROL | MOD_ALT | MOD_COMMAND;

/// `VKEY_PROCESSKEY`: what Chromium reports (as `keyCode` 229) for keys whose
/// text arrives separately, as with an IME. Pages treat it as "wait for the
/// input event" rather than as a shortcut.
const VK_PROCESSKEY: i32 = 0xE5;

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
    /// The text the key types with the current modifiers (`"Z"`, `"?"`,
    /// `"é"`), when it types any.
    pub text: Option<String>,
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

/// Characters a US keyboard types with Shift held. Chromium derives
/// `KeyboardEvent.key` from the key code plus the Shift flag, so the flag has
/// to be set even when the platform folded Shift into the key name.
fn is_shifted(c: char) -> bool {
    c.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(c)
}

impl KeyInput {
    /// Whether this key can be sent to Chromium as key down/up events.
    pub fn is_known(&self) -> bool {
        self.codes().is_some()
    }

    /// `(windows key code, XKB keycode, character for CHAR events, extra
    /// modifiers)`.
    fn codes(&self) -> Option<(i32, i32, Option<u16>, u32)> {
        let typed = self
            .text
            .as_deref()
            .filter(|t| t.chars().count() == 1)
            .and_then(|t| t.chars().next());
        // A character outside the US layout (é, ü, ß, ...): no key code
        // names it (the key name may be the US key at that position), so
        // report it the way an IME key is reported. The page still sees
        // keydown/keyup; the text arrives as input.
        if typed.is_some_and(|c| !c.is_ascii() && !c.is_control()) {
            return Some((VK_PROCESSKEY, 0, None, 0));
        }
        if let Some((vk, native, chr)) = lookup(&self.key) {
            let shifted = typed.or_else(|| {
                let mut chars = self.key.chars();
                chars.next().filter(|_| chars.next().is_none())
            });
            let extra = if shifted.is_some_and(is_shifted) { MOD_SHIFT } else { 0 };
            return Some((vk, native, chr, extra));
        }
        None
    }

    /// The `(windows key code, MOD_KEYS modifiers)` Chromium will report back
    /// for this key, e.g. in an unhandled-key notification.
    pub fn signature(&self) -> Option<(i32, u32)> {
        self.codes()
            .map(|(vk, _, _, extra)| (vk, (self.modifiers | extra) & MOD_KEYS))
    }

    pub(crate) fn to_cef_events(&self) -> Vec<KeyEvent> {
        let Some((vk, native, chr, extra)) = self.codes() else {
            return Vec::new();
        };
        let mut modifiers = self.modifiers | extra;
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

    fn down(key: &str, text: Option<&str>, modifiers: u32) -> KeyInput {
        KeyInput {
            key: key.into(),
            text: text.map(str::to_string),
            phase: KeyPhase::Down,
            modifiers,
            repeat: false,
        }
    }

    #[test]
    fn enter_emits_char() {
        let ev = down("enter", None, 0).to_cef_events();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].character, b'\r' as u16);
    }

    #[test]
    fn unknown_keys_are_dropped() {
        assert!(lookup("hyper-whatever").is_none());
        assert!(down("hyper-whatever", None, 0).to_cef_events().is_empty());
    }

    #[test]
    fn shifted_symbols_carry_shift() {
        // GPUI reports Shift+/ as "?" without the shift flag.
        let ev = down("?", Some("?"), 0).to_cef_events();
        assert_eq!(ev[0].windows_key_code, 0xBF);
        assert_eq!(ev[0].modifiers & MOD_SHIFT, MOD_SHIFT);
        // An upper-case letter whose shift was folded into the text.
        let ev = down("z", Some("Z"), 0).to_cef_events();
        assert_eq!(ev[0].windows_key_code, 0x5A);
        assert_eq!(ev[0].modifiers & MOD_SHIFT, MOD_SHIFT);
        // Plain keys stay unshifted.
        let ev = down("/", Some("/"), 0).to_cef_events();
        assert_eq!(ev[0].modifiers & MOD_SHIFT, 0);
        assert_eq!(down("a", Some("a"), MOD_CONTROL).signature(), Some((0x41, MOD_CONTROL)));
    }

    #[test]
    fn non_us_characters_still_send_key_events() {
        // The key name may be the US key at that position.
        assert_eq!(down("[", Some("ü"), 0).signature(), Some((VK_PROCESSKEY, 0)));
        let ev = down("eacute", Some("é"), 0).to_cef_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].type_, KeyEventType::RAWKEYDOWN);
        assert_eq!(ev[0].windows_key_code, VK_PROCESSKEY);
        let mut up = down("eacute", Some("é"), 0);
        up.phase = KeyPhase::Up;
        assert_eq!(up.to_cef_events()[0].type_, KeyEventType::KEYUP);
    }
}
