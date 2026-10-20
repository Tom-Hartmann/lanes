//! Parsing and formatting of key combinations such as `Ctrl+Shift+F9`.
//!
//! Combos are stored in settings as human-readable strings and converted to
//! Win32 virtual-key codes here. This module is pure logic and fully tested.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A key combination: any set of modifiers plus exactly one non-modifier key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Win32 virtual-key code of the main key.
    pub vk: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComboParseError {
    #[error("the combo is empty")]
    Empty,
    #[error("unknown key \"{0}\"")]
    UnknownKey(String),
    #[error("the combo needs a key besides the modifiers")]
    MissingKey,
    #[error("the combo has more than one key (\"{0}\" and \"{1}\")")]
    MultipleKeys(String, String),
}

/// Virtual-key names, in the spelling used for display. Lookup is case-insensitive.
const NAMED_KEYS: &[(&str, u16)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Pause", 0x13),
    ("CapsLock", 0x14),
    ("Esc", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("PrintScreen", 0x2C),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("NumpadMultiply", 0x6A),
    ("NumpadAdd", 0x6B),
    ("NumpadSubtract", 0x6D),
    ("NumpadDecimal", 0x6E),
    ("NumpadDivide", 0x6F),
    ("ScrollLock", 0x91),
    ("Semicolon", 0xBA),
    ("Equal", 0xBB),
    ("Comma", 0xBC),
    ("Minus", 0xBD),
    ("Period", 0xBE),
    ("Slash", 0xBF),
    ("Backquote", 0xC0),
    ("BracketLeft", 0xDB),
    ("Backslash", 0xDC),
    ("BracketRight", 0xDD),
    ("Quote", 0xDE),
];

const ALIASES: &[(&str, u16)] = &[
    ("Escape", 0x1B),
    ("Return", 0x0D),
    ("Del", 0x2E),
    ("Ins", 0x2D),
    ("PgUp", 0x21),
    ("PgDn", 0x22),
    ("PrtSc", 0x2C),
    ("ArrowLeft", 0x25),
    ("ArrowUp", 0x26),
    ("ArrowRight", 0x27),
    ("ArrowDown", 0x28),
];

fn key_from_name(name: &str) -> Option<u16> {
    let lookup = |table: &[(&str, u16)]| {
        table.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, vk)| vk)
    };
    if let Some(vk) = lookup(NAMED_KEYS).or_else(|| lookup(ALIASES)) {
        return Some(vk);
    }
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    // Letters and digits map to their ASCII codes.
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Some(bytes[0] as u16);
    }
    // F1..F24 -> 0x70..0x87.
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then(|| 0x70 + n - 1);
    }
    // Numpad0..Numpad9 -> 0x60..0x69.
    if let Some(n) = upper.strip_prefix("NUMPAD").and_then(|n| n.parse::<u16>().ok()) {
        return (n <= 9).then(|| 0x60 + n);
    }
    None
}

fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => NAMED_KEYS
            .iter()
            .find(|&&(_, v)| v == vk)
            .map(|(n, _)| (*n).to_string())
            .unwrap_or_else(|| format!("VK{vk:#04X}")),
    }
}

impl FromStr for Combo {
    type Err = ComboParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(ComboParseError::Empty);
        }
        let mut combo = Combo { ctrl: false, alt: false, shift: false, win: false, vk: 0 };
        let mut key: Option<&str> = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => combo.ctrl = true,
                "alt" => combo.alt = true,
                "shift" => combo.shift = true,
                "win" | "super" | "meta" => combo.win = true,
                // "Ctrl++" splits into an empty trailing part; treat it like the Equal/Plus key.
                "" => return Err(ComboParseError::UnknownKey("+".into())),
                _ => {
                    if let Some(prev) = key {
                        return Err(ComboParseError::MultipleKeys(prev.into(), part.into()));
                    }
                    combo.vk = key_from_name(part)
                        .ok_or_else(|| ComboParseError::UnknownKey(part.into()))?;
                    key = Some(part);
                }
            }
        }
        if key.is_none() {
            return Err(ComboParseError::MissingKey);
        }
        Ok(combo)
    }
}

impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.vk))
    }
}

impl Serialize for Combo {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Combo {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Combo {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn parses_modifiers_and_key() {
        let c = parse("Ctrl+Shift+F9");
        assert!(c.ctrl && c.shift && !c.alt && !c.win);
        assert_eq!(c.vk, 0x78);
    }

    #[test]
    fn is_case_and_space_insensitive() {
        assert_eq!(parse(" ctrl + ALT + f10 "), parse("Ctrl+Alt+F10"));
        assert_eq!(parse("control+escape"), parse("Ctrl+Esc"));
    }

    #[test]
    fn maps_key_families() {
        assert_eq!(parse("A").vk, 0x41);
        assert_eq!(parse("7").vk, 0x37);
        assert_eq!(parse("F1").vk, 0x70);
        assert_eq!(parse("F24").vk, 0x87);
        assert_eq!(parse("Numpad0").vk, 0x60);
        assert_eq!(parse("Numpad9").vk, 0x69);
        assert_eq!(parse("ArrowUp").vk, 0x26);
    }

    #[test]
    fn display_roundtrips() {
        for s in ["Ctrl+Alt+Shift+Win+F12", "Alt+Z", "Numpad5", "Ctrl+PageDown", "Shift+Backquote"] {
            assert_eq!(parse(s).to_string(), s);
            assert_eq!(parse(&parse(s).to_string()), parse(s));
        }
    }

    #[test]
    fn display_uses_canonical_modifier_order() {
        assert_eq!(parse("Shift+Win+Alt+Ctrl+K").to_string(), "Ctrl+Alt+Shift+Win+K");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("".parse::<Combo>(), Err(ComboParseError::Empty));
        assert_eq!("Ctrl+Alt".parse::<Combo>(), Err(ComboParseError::MissingKey));
        assert_eq!("Ctrl+Foo".parse::<Combo>(), Err(ComboParseError::UnknownKey("Foo".into())));
        assert_eq!("F25".parse::<Combo>(), Err(ComboParseError::UnknownKey("F25".into())));
        assert_eq!(
            "Ctrl+A+B".parse::<Combo>(),
            Err(ComboParseError::MultipleKeys("A".into(), "B".into()))
        );
    }

    #[test]
    fn serde_uses_the_string_form() {
        let c = parse("Alt+F10");
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"Alt+F10\"");
        assert_eq!(serde_json::from_str::<Combo>(&json).unwrap(), c);
        assert!(serde_json::from_str::<Combo>("\"Alt\"").is_err());
    }
}
