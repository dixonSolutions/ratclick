//! Keyboard accelerator parsing and conversion.
//!
//! RatClick has to speak two dialects of "keyboard shortcut":
//!
//! * GTK/GNOME accelerator strings — `<Super><Shift>c`, `<Control><Alt>Delete`
//! * keyd bindings — `M-S-c`, `C-A-delete`
//!
//! [`Accel`] is the neutral representation both are parsed into, so conflict
//! detection can compare shortcuts that were written in different dialects.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Modifier keys, stored as a small bitset so ordering never matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    pub const CONTROL: Modifiers = Modifiers(1 << 1);
    pub const ALT: Modifiers = Modifiers(1 << 2);
    pub const SUPER: Modifiers = Modifiers(1 << 3);

    pub fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccelError {
    #[error("shortcut is empty")]
    Empty,
    #[error("shortcut `{0}` has no non-modifier key")]
    NoKey(String),
    #[error("unknown modifier `<{0}>`")]
    UnknownModifier(String),
    #[error("unterminated `<` in shortcut `{0}`")]
    Unterminated(String),
    #[error("`{0}` is not a key RatClick knows how to bind")]
    UnknownKey(String),
    #[error("`{0}` needs at least one modifier — a bare key would swallow normal typing")]
    NeedsModifier(String),
}

/// A parsed, dialect-neutral keyboard shortcut.
///
/// The `key` is always stored in canonical lowercase evdev-ish form (`c`, `f9`,
/// `space`, `delete`), which is what keyd expects and what we can map back to a
/// GTK keyname on demand.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Accel {
    pub mods: Modifiers,
    pub key: String,
}

impl Accel {
    pub fn new(mods: Modifiers, key: impl Into<String>) -> Self {
        Accel {
            mods,
            key: key.into(),
        }
    }

    /// Parse a GTK/GNOME accelerator string such as `<Super><Shift>c`.
    ///
    /// Also accepts keyd-style input (`M-S-c`) so that user-typed config is
    /// forgiving; the two syntaxes are unambiguous because GTK accelerators
    /// always use angle brackets.
    pub fn parse(input: &str) -> Result<Self, AccelError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(AccelError::Empty);
        }
        if trimmed.contains('<') {
            Self::parse_gtk(trimmed)
        } else if trimmed.contains('-') && trimmed.len() > 1 {
            Self::parse_keyd(trimmed)
        } else {
            // A bare keyname like `F9`.
            Self::finish(Modifiers::NONE, trimmed)
        }
    }

    fn parse_gtk(input: &str) -> Result<Self, AccelError> {
        let mut mods = Modifiers::NONE;
        let mut rest = input;

        while let Some(open) = rest.find('<') {
            // Anything before a `<` that isn't whitespace is malformed, but GTK
            // tolerates it, so we do too and treat it as part of the key.
            if open != 0 {
                break;
            }
            let close = rest
                .find('>')
                .ok_or_else(|| AccelError::Unterminated(input.to_string()))?;
            let name = &rest[open + 1..close];
            mods |= modifier_from_gtk(name)
                .ok_or_else(|| AccelError::UnknownModifier(name.to_string()))?;
            rest = &rest[close + 1..];
        }

        if rest.is_empty() {
            return Err(AccelError::NoKey(input.to_string()));
        }
        Self::finish(mods, rest)
    }

    fn parse_keyd(input: &str) -> Result<Self, AccelError> {
        let mut mods = Modifiers::NONE;
        let mut rest = input;

        // keyd prefixes are single letters followed by `-`. A trailing `-` key
        // (the minus key itself) is written `-` and must not be eaten as a
        // separator, hence the `rest.len() > 2` guard.
        while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
            let m = match rest.as_bytes()[0] {
                b'C' => Modifiers::CONTROL,
                b'S' => Modifiers::SHIFT,
                b'A' => Modifiers::ALT,
                b'M' | b'G' => Modifiers::SUPER,
                _ => break,
            };
            mods |= m;
            rest = &rest[2..];
        }

        if rest.is_empty() {
            return Err(AccelError::NoKey(input.to_string()));
        }
        Self::finish(mods, rest)
    }

    fn finish(mods: Modifiers, key: &str) -> Result<Self, AccelError> {
        let key = canonical_key(key).ok_or_else(|| AccelError::UnknownKey(key.to_string()))?;
        Ok(Accel { mods, key })
    }

    /// Reject shortcuts that would make the machine unusable.
    ///
    /// A binding with no modifiers steals a key from every application, so we
    /// only allow it for keys that are not part of ordinary typing (function
    /// keys and the dedicated multimedia keys).
    pub fn validate(&self) -> Result<(), AccelError> {
        if !self.mods.is_empty() {
            return Ok(());
        }
        let standalone_ok = self.key.starts_with('f')
            && self.key[1..].chars().all(|c| c.is_ascii_digit())
            && self.key.len() > 1;
        if standalone_ok || MEDIA_KEYS.contains(&self.key.as_str()) {
            Ok(())
        } else {
            Err(AccelError::NeedsModifier(self.to_gtk()))
        }
    }

    /// Render as a GTK/GNOME accelerator string, e.g. `<Super><Shift>c`.
    pub fn to_gtk(&self) -> String {
        let mut out = String::new();
        // GTK does not care about modifier order, but a stable order makes the
        // strings we write into gsettings comparable by eye.
        if self.mods.contains(Modifiers::CONTROL) {
            out.push_str("<Control>");
        }
        if self.mods.contains(Modifiers::ALT) {
            out.push_str("<Alt>");
        }
        if self.mods.contains(Modifiers::SHIFT) {
            out.push_str("<Shift>");
        }
        if self.mods.contains(Modifiers::SUPER) {
            out.push_str("<Super>");
        }
        out.push_str(&gtk_key_name(&self.key));
        out
    }

    /// The keyd *layer* this accelerator's key must be bound in.
    ///
    /// keyd does not accept modifier prefixes on the left-hand side of a
    /// mapping — `M-S-f12 = …` is rejected outright with "not a valid key or
    /// alias". Modified keys are instead expressed by putting the bare key in a
    /// modifier layer:
    ///
    /// ```text
    /// [meta+shift]
    /// f12 = command(…)
    /// ```
    ///
    /// Returns `None` for an unmodified accelerator, which belongs in `[main]`.
    pub fn to_keyd_layer(&self) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        if self.mods.contains(Modifiers::CONTROL) {
            parts.push("control");
        }
        if self.mods.contains(Modifiers::ALT) {
            parts.push("alt");
        }
        if self.mods.contains(Modifiers::SHIFT) {
            parts.push("shift");
        }
        if self.mods.contains(Modifiers::SUPER) {
            parts.push("meta");
        }
        (!parts.is_empty()).then(|| parts.join("+"))
    }

    /// Reconstruct the modifiers implied by a keyd layer name.
    ///
    /// Returns `None` for a layer that is not built purely out of modifier
    /// layers — a user-defined layer like `[nav]` is only active while some
    /// other key is held, so a binding inside it is not a global shortcut and
    /// cannot collide with one.
    pub fn modifiers_from_keyd_layer(layer: &str) -> Option<Modifiers> {
        if layer == "main" {
            return Some(Modifiers::NONE);
        }
        let mut mods = Modifiers::NONE;
        for part in layer.split('+') {
            mods |= match part.trim() {
                "control" | "ctrl" | "C" => Modifiers::CONTROL,
                "shift" | "S" => Modifiers::SHIFT,
                "alt" | "A" => Modifiers::ALT,
                "meta" | "super" | "M" | "G" => Modifiers::SUPER,
                _ => return None,
            };
        }
        Some(mods)
    }

    /// Render in keyd's *macro* notation, e.g. `M-S-c`.
    ///
    /// This is the form keyd accepts on the right-hand side of a mapping, and
    /// what `keyd monitor` prints. It is **not** valid on the left-hand side;
    /// use [`Accel::to_keyd_layer`] plus the bare [`Accel::key`] for that.
    pub fn to_keyd(&self) -> String {
        let mut out = String::new();
        if self.mods.contains(Modifiers::CONTROL) {
            out.push_str("C-");
        }
        if self.mods.contains(Modifiers::ALT) {
            out.push_str("A-");
        }
        if self.mods.contains(Modifiers::SHIFT) {
            out.push_str("S-");
        }
        if self.mods.contains(Modifiers::SUPER) {
            out.push_str("M-");
        }
        out.push_str(&self.key);
        out
    }

    /// Human-readable label for the GUI, e.g. `Super+Shift+C`.
    pub fn to_display(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.contains(Modifiers::CONTROL) {
            parts.push("Ctrl".into());
        }
        if self.mods.contains(Modifiers::ALT) {
            parts.push("Alt".into());
        }
        if self.mods.contains(Modifiers::SHIFT) {
            parts.push("Shift".into());
        }
        if self.mods.contains(Modifiers::SUPER) {
            parts.push("Super".into());
        }
        parts.push(display_key_name(&self.key));
        parts.join("+")
    }
}

impl fmt::Display for Accel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_gtk())
    }
}

impl FromStr for Accel {
    type Err = AccelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Accel::parse(s)
    }
}

impl Serialize for Accel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_gtk())
    }
}

impl<'de> Deserialize<'de> for Accel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Accel::parse(&raw).map_err(serde::de::Error::custom)
    }
}

fn modifier_from_gtk(name: &str) -> Option<Modifiers> {
    match name.to_ascii_lowercase().as_str() {
        "shift" | "shft" => Some(Modifiers::SHIFT),
        // GTK's `<Primary>` is Control on Linux.
        "control" | "ctrl" | "ctl" | "primary" => Some(Modifiers::CONTROL),
        "alt" | "mod1" | "meta" => Some(Modifiers::ALT),
        "super" | "mod4" | "hyper" | "logo" | "win" => Some(Modifiers::SUPER),
        // `<Release>` appears in some GNOME accels; it does not affect matching.
        "release" => Some(Modifiers::NONE),
        _ => None,
    }
}

const MEDIA_KEYS: &[&str] = &[
    "playpause",
    "stopcd",
    "nextsong",
    "previoussong",
    "mute",
    "volumeup",
    "volumedown",
    "brightnessup",
    "brightnessdown",
    "prog1",
    "prog2",
    "micmute",
    "calc",
    "search",
];

/// Map a key name written in any of the dialects onto the canonical keyd name.
fn canonical_key(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }

    let mapped = match lower.as_str() {
        // GTK keysym names -> keyd names.
        "return" | "enter" | "kp_enter" => "enter",
        "escape" | "esc" => "esc",
        "backspace" => "backspace",
        "delete" | "del" => "delete",
        "insert" | "ins" => "insert",
        "page_up" | "prior" | "pageup" => "pageup",
        "page_down" | "next" | "pagedown" => "pagedown",
        "home" => "home",
        "end" => "end",
        "up" => "up",
        "down" => "down",
        "left" => "left",
        "right" => "right",
        "tab" => "tab",
        "space" | "spacebar" => "space",
        "period" | "." => "dot",
        "comma" | "," => "comma",
        "minus" | "-" => "minus",
        "equal" | "=" => "equal",
        "slash" | "/" => "slash",
        "backslash" | "\\" => "backslash",
        "semicolon" | ";" => "semicolon",
        "apostrophe" | "'" => "apostrophe",
        "grave" | "`" => "grave",
        "bracketleft" | "[" => "leftbrace",
        "bracketright" | "]" => "rightbrace",
        "audiomute" => "mute",
        "audioraisevolume" => "volumeup",
        "audiolowervolume" => "volumedown",
        "audioplay" => "playpause",
        "audionext" => "nextsong",
        "audioprev" => "previoussong",
        other => other,
    };

    let ok = mapped.len() == 1 && mapped.chars().next().unwrap().is_ascii_alphanumeric()
        || (mapped.starts_with('f')
            && mapped.len() > 1
            && mapped[1..].chars().all(|c| c.is_ascii_digit()))
        || matches!(
            mapped,
            "enter"
                | "esc"
                | "backspace"
                | "delete"
                | "insert"
                | "pageup"
                | "pagedown"
                | "home"
                | "end"
                | "up"
                | "down"
                | "left"
                | "right"
                | "tab"
                | "space"
                | "dot"
                | "comma"
                | "minus"
                | "equal"
                | "slash"
                | "backslash"
                | "semicolon"
                | "apostrophe"
                | "grave"
                | "leftbrace"
                | "rightbrace"
        )
        || MEDIA_KEYS.contains(&mapped);

    ok.then(|| mapped.to_string())
}

/// Inverse of [`canonical_key`] for the subset GTK spells differently.
fn gtk_key_name(key: &str) -> String {
    match key {
        "enter" => "Return".into(),
        "esc" => "Escape".into(),
        "delete" => "Delete".into(),
        "insert" => "Insert".into(),
        "pageup" => "Page_Up".into(),
        "pagedown" => "Page_Down".into(),
        "backspace" => "BackSpace".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "tab" => "Tab".into(),
        "space" => "space".into(),
        "dot" => "period".into(),
        "comma" => "comma".into(),
        "minus" => "minus".into(),
        "equal" => "equal".into(),
        "slash" => "slash".into(),
        "backslash" => "backslash".into(),
        "semicolon" => "semicolon".into(),
        "apostrophe" => "apostrophe".into(),
        "grave" => "grave".into(),
        "leftbrace" => "bracketleft".into(),
        "rightbrace" => "bracketright".into(),
        "mute" => "AudioMute".into(),
        "volumeup" => "AudioRaiseVolume".into(),
        "volumedown" => "AudioLowerVolume".into(),
        "playpause" => "AudioPlay".into(),
        k if k.starts_with('f') && k.len() > 1 && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            k.to_uppercase()
        }
        k => k.to_string(),
    }
}

fn display_key_name(key: &str) -> String {
    match key {
        "esc" => "Esc".into(),
        "pageup" => "Page Up".into(),
        "pagedown" => "Page Down".into(),
        "leftbrace" => "[".into(),
        "rightbrace" => "]".into(),
        "dot" => ".".into(),
        "comma" => ",".into(),
        "minus" => "-".into(),
        "equal" => "=".into(),
        "slash" => "/".into(),
        "backslash" => "\\".into(),
        "semicolon" => ";".into(),
        "apostrophe" => "'".into(),
        "grave" => "`".into(),
        k if k.len() == 1 => k.to_uppercase(),
        k if k.starts_with('f') && k.len() > 1 && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            k.to_uppercase()
        }
        k => {
            let mut chars = k.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gtk_accelerators() {
        let a = Accel::parse("<Super><Shift>c").unwrap();
        assert_eq!(a.key, "c");
        assert!(a.mods.contains(Modifiers::SUPER));
        assert!(a.mods.contains(Modifiers::SHIFT));
        assert!(!a.mods.contains(Modifiers::CONTROL));
    }

    #[test]
    fn parses_keyd_bindings() {
        let a = Accel::parse("M-S-c").unwrap();
        assert_eq!(a, Accel::parse("<Super><Shift>c").unwrap());
    }

    #[test]
    fn modifier_order_does_not_matter() {
        assert_eq!(
            Accel::parse("<Shift><Super>c").unwrap(),
            Accel::parse("<Super><Shift>c").unwrap()
        );
    }

    #[test]
    fn primary_is_control() {
        assert_eq!(
            Accel::parse("<Primary>k").unwrap(),
            Accel::parse("<Control>k").unwrap()
        );
    }

    #[test]
    fn gtk_roundtrip_is_stable() {
        for s in ["<Control><Alt>Delete", "<Super>F9", "<Shift><Super>space"] {
            let a = Accel::parse(s).unwrap();
            let b = Accel::parse(&a.to_gtk()).unwrap();
            assert_eq!(a, b, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn keyd_roundtrip_is_stable() {
        let a = Accel::parse("<Control><Alt>Delete").unwrap();
        assert_eq!(a.to_keyd(), "C-A-delete");
        assert_eq!(Accel::parse("C-A-delete").unwrap(), a);
    }

    #[test]
    fn function_keys_may_stand_alone_but_letters_may_not() {
        assert!(Accel::parse("F9").unwrap().validate().is_ok());
        assert!(Accel::parse("c").unwrap().validate().is_err());
        assert!(Accel::parse("<Super>c").unwrap().validate().is_ok());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Accel::parse("").is_err());
        assert!(Accel::parse("<Super>").is_err());
        assert!(Accel::parse("<Nonsense>c").is_err());
        assert!(Accel::parse("<Super>notakey").is_err());
    }

    #[test]
    fn display_is_human_readable() {
        assert_eq!(
            Accel::parse("<Super><Shift>c").unwrap().to_display(),
            "Shift+Super+C"
        );
    }

    #[test]
    fn minus_key_is_not_mistaken_for_a_keyd_separator() {
        let a = Accel::parse("<Control>minus").unwrap();
        assert_eq!(a.key, "minus");
        assert_eq!(a.to_keyd(), "C-minus");
        assert_eq!(Accel::parse("C-minus").unwrap(), a);
    }
}
