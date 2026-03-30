use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const KITTY_FLAG_REPORT_EVENT_TYPES: u16 = 0b0000_0010;
const KITTY_FLAG_REPORT_ALTERNATE_KEYS: u16 = 0b0000_0100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalKey {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub kind: crossterm::event::KeyEventKind,
    pub shifted_codepoint: Option<u32>,
}

impl TerminalKey {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            shifted_codepoint: None,
        }
    }

    pub fn with_kind(mut self, kind: crossterm::event::KeyEventKind) -> Self {
        self.kind = kind;
        self
    }

    #[allow(dead_code)] // Reserved for the upcoming raw input parser to preserve shifted/base key pairs.
    pub fn with_shifted_codepoint(mut self, shifted_codepoint: u32) -> Self {
        self.shifted_codepoint = Some(shifted_codepoint);
        self
    }

    pub fn as_key_event(self) -> KeyEvent {
        KeyEvent::new_with_kind(self.code, self.modifiers, self.kind)
    }
}

impl From<KeyEvent> for TerminalKey {
    fn from(value: KeyEvent) -> Self {
        Self::new(value.code, value.modifiers).with_kind(value.kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardProtocol {
    Legacy,
    Kitty { flags: u16 },
}

impl KeyboardProtocol {
    pub fn from_kitty_flags(flags: u16) -> Self {
        if flags == 0 {
            Self::Legacy
        } else {
            Self::Kitty { flags }
        }
    }
}

#[allow(dead_code)] // Next step: raw stdin parser will feed TerminalKey directly through this path.
pub fn parse_terminal_key_sequence(data: &str) -> Option<TerminalKey> {
    parse_kitty_key_sequence(data)
        .or_else(|| parse_modify_other_keys_sequence(data))
        .or_else(|| parse_legacy_key_sequence(data))
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn parse_kitty_key_sequence(data: &str) -> Option<TerminalKey> {
    let body = data.strip_prefix("\x1b[")?.strip_suffix('u')?;

    let (main, event_type) = match body.rsplit_once(':') {
        Some((head, tail)) if tail.chars().all(|ch| ch.is_ascii_digit()) && head.contains(';') => {
            (head, Some(tail))
        }
        _ => (body, None),
    };

    let (key_part, modifier_part) = main.rsplit_once(';').unwrap_or((main, "1"));
    let modifier = modifier_part.parse::<u8>().ok()?.checked_sub(1)?;

    let mut key_fields = key_part.split(':');
    let codepoint = key_fields.next()?.parse::<u32>().ok()?;
    let shifted_codepoint = key_fields
        .next()
        .filter(|field| !field.is_empty())
        .and_then(|field| field.parse::<u32>().ok());

    let code = kitty_codepoint_to_keycode(codepoint)?;
    let kind = parse_kitty_event_type(event_type)?;

    Some(TerminalKey {
        code,
        modifiers: key_modifiers_from_u8(modifier),
        kind,
        shifted_codepoint,
    })
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn parse_modify_other_keys_sequence(data: &str) -> Option<TerminalKey> {
    let body = data.strip_prefix("\x1b[27;")?.strip_suffix('~')?;
    let (modifier_part, codepoint_part) = body.split_once(';')?;
    let modifier = modifier_part.parse::<u8>().ok()?.checked_sub(1)?;
    let codepoint = codepoint_part.parse::<u32>().ok()?;

    Some(TerminalKey::new(
        kitty_codepoint_to_keycode(codepoint)?,
        key_modifiers_from_u8(modifier),
    ))
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn parse_legacy_key_sequence(data: &str) -> Option<TerminalKey> {
    if let Some(key) = parse_legacy_special_sequence(data) {
        return Some(key);
    }

    match data {
        "\r" | "\n" => Some(TerminalKey::new(KeyCode::Enter, KeyModifiers::empty())),
        "\t" => Some(TerminalKey::new(KeyCode::Tab, KeyModifiers::empty())),
        "\x1b" => Some(TerminalKey::new(KeyCode::Esc, KeyModifiers::empty())),
        "\x7f" => Some(TerminalKey::new(KeyCode::Backspace, KeyModifiers::empty())),
        _ if data.starts_with('\x1b') => {
            let rest = data.strip_prefix('\x1b')?;
            if rest.chars().count() == 1 {
                let ch = rest.chars().next()?;
                Some(TerminalKey::new(KeyCode::Char(ch), KeyModifiers::ALT))
            } else {
                None
            }
        }
        _ if data.chars().count() == 1 => {
            let ch = data.chars().next()?;

            if let Some(ctrl_key) = parse_legacy_ctrl_char(ch) {
                return Some(ctrl_key);
            }

            let mut modifiers = KeyModifiers::empty();
            let code = if ch.is_ascii_uppercase() {
                modifiers |= KeyModifiers::SHIFT;
                KeyCode::Char(ch)
            } else {
                KeyCode::Char(ch)
            };
            Some(TerminalKey::new(code, modifiers))
        }
        _ => None,
    }
}

fn parse_legacy_ctrl_char(ch: char) -> Option<TerminalKey> {
    match ch as u32 {
        0 => Some(TerminalKey::new(KeyCode::Char(' '), KeyModifiers::CONTROL)),
        1..=26 => Some(TerminalKey::new(
            KeyCode::Char(char::from_u32((ch as u32) + 96)?),
            KeyModifiers::CONTROL,
        )),
        27 => Some(TerminalKey::new(KeyCode::Char('['), KeyModifiers::CONTROL)),
        28 => Some(TerminalKey::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
        29 => Some(TerminalKey::new(KeyCode::Char(']'), KeyModifiers::CONTROL)),
        30 => Some(TerminalKey::new(KeyCode::Char('^'), KeyModifiers::CONTROL)),
        31 => Some(TerminalKey::new(KeyCode::Char('-'), KeyModifiers::CONTROL)),
        _ => None,
    }
}

fn parse_legacy_special_sequence(data: &str) -> Option<TerminalKey> {
    match data {
        "\x1b\x1b[A" => Some(TerminalKey::new(KeyCode::Up, KeyModifiers::ALT)),
        "\x1b\x1b[B" => Some(TerminalKey::new(KeyCode::Down, KeyModifiers::ALT)),
        "\x1b\x1b[C" => Some(TerminalKey::new(KeyCode::Right, KeyModifiers::ALT)),
        "\x1b\x1b[D" => Some(TerminalKey::new(KeyCode::Left, KeyModifiers::ALT)),
        "\x1b[A" => Some(TerminalKey::new(KeyCode::Up, KeyModifiers::empty())),
        "\x1b[B" => Some(TerminalKey::new(KeyCode::Down, KeyModifiers::empty())),
        "\x1b[C" => Some(TerminalKey::new(KeyCode::Right, KeyModifiers::empty())),
        "\x1b[D" => Some(TerminalKey::new(KeyCode::Left, KeyModifiers::empty())),
        "\x1b[H" | "\x1bOH" | "\x1b[1~" | "\x1b[7~" => {
            Some(TerminalKey::new(KeyCode::Home, KeyModifiers::empty()))
        }
        "\x1b[F" | "\x1bOF" | "\x1b[4~" | "\x1b[8~" => {
            Some(TerminalKey::new(KeyCode::End, KeyModifiers::empty()))
        }
        "\x1b[5~" => Some(TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty())),
        "\x1b[6~" => Some(TerminalKey::new(KeyCode::PageDown, KeyModifiers::empty())),
        "\x1b[2~" => Some(TerminalKey::new(KeyCode::Insert, KeyModifiers::empty())),
        "\x1b[3~" => Some(TerminalKey::new(KeyCode::Delete, KeyModifiers::empty())),
        "\x1b[Z" => Some(TerminalKey::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        _ => parse_xterm_modified_special_sequence(data),
    }
}

fn parse_xterm_modified_special_sequence(data: &str) -> Option<TerminalKey> {
    let body = data.strip_prefix("\x1b[")?;

    if let Some(body) = body.strip_prefix("1;") {
        let suffix_char = body.chars().last()?;
        if suffix_char.is_ascii_alphabetic() {
            let modifier = body.strip_suffix(suffix_char)?;
            let mod_value = modifier.parse::<u8>().ok()?.checked_sub(1)?;
            let code = match suffix_char {
                'A' => KeyCode::Up,
                'B' => KeyCode::Down,
                'C' => KeyCode::Right,
                'D' => KeyCode::Left,
                'H' => KeyCode::Home,
                'F' => KeyCode::End,
                _ => return None,
            };
            return Some(TerminalKey::new(code, key_modifiers_from_u8(mod_value)));
        }
    }

    let tilde_body = body.strip_suffix('~')?;
    let (code_part, modifier_part) = tilde_body.split_once(';')?;
    let mod_value = modifier_part.parse::<u8>().ok()?.checked_sub(1)?;
    let code = match code_part {
        "2" => KeyCode::Insert,
        "3" => KeyCode::Delete,
        "5" => KeyCode::PageUp,
        "6" => KeyCode::PageDown,
        _ => return None,
    };
    Some(TerminalKey::new(code, key_modifiers_from_u8(mod_value)))
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn parse_kitty_event_type(value: Option<&str>) -> Option<crossterm::event::KeyEventKind> {
    match value.unwrap_or("1") {
        "1" => Some(crossterm::event::KeyEventKind::Press),
        "2" => Some(crossterm::event::KeyEventKind::Repeat),
        "3" => Some(crossterm::event::KeyEventKind::Release),
        _ => None,
    }
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn kitty_codepoint_to_keycode(codepoint: u32) -> Option<KeyCode> {
    match codepoint {
        8 | 127 => Some(KeyCode::Backspace),
        9 => Some(KeyCode::Tab),
        13 | 57414 => Some(KeyCode::Enter),
        27 => Some(KeyCode::Esc),
        57417 => Some(KeyCode::Left),
        57418 => Some(KeyCode::Right),
        57419 => Some(KeyCode::Up),
        57420 => Some(KeyCode::Down),
        57421 => Some(KeyCode::PageUp),
        57422 => Some(KeyCode::PageDown),
        57423 => Some(KeyCode::Home),
        57424 => Some(KeyCode::End),
        57425 => Some(KeyCode::Insert),
        57426 => Some(KeyCode::Delete),
        value => char::from_u32(value).map(KeyCode::Char),
    }
}

#[allow(dead_code)] // Reserved for the upcoming raw stdin parser.
fn key_modifiers_from_u8(modifier: u8) -> KeyModifiers {
    let mut mods = KeyModifiers::empty();
    if modifier & 0b0000_0001 != 0 {
        mods |= KeyModifiers::SHIFT;
    }
    if modifier & 0b0000_0010 != 0 {
        mods |= KeyModifiers::ALT;
    }
    if modifier & 0b0000_0100 != 0 {
        mods |= KeyModifiers::CONTROL;
    }
    if modifier & 0b0000_1000 != 0 {
        mods |= KeyModifiers::SUPER;
    }
    if modifier & 0b0001_0000 != 0 {
        mods |= KeyModifiers::HYPER;
    }
    if modifier & 0b0010_0000 != 0 {
        mods |= KeyModifiers::META;
    }
    mods
}

/// Encode a key event for a PTY child using the pane's negotiated keyboard protocol.
pub fn encode_key(key: KeyEvent, protocol: KeyboardProtocol) -> Vec<u8> {
    encode_terminal_key(key.into(), protocol)
}

pub fn encode_terminal_key(key: TerminalKey, protocol: KeyboardProtocol) -> Vec<u8> {
    if let KeyboardProtocol::Kitty { flags } = protocol {
        if let Some(bytes) = try_encode_csi_u(&key, flags) {
            return bytes;
        }
    }
    encode_legacy(key.as_key_event())
}

/// CSI u encoding: \e[{codepoint};{modifiers}u
/// Used when the child has pushed Kitty keyboard enhancement.
/// Returns None if the key doesn't need CSI u (unmodified basic keys).
fn try_encode_csi_u(key: &TerminalKey, flags: u16) -> Option<Vec<u8>> {
    let mods = key.modifiers;

    // Unmodified keys use legacy encoding (more compatible)
    if mods.is_empty() {
        return None;
    }

    // Plain Ctrl+letter is well-represented in legacy (bytes 1-26)
    if mods == KeyModifiers::CONTROL {
        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_alphabetic() {
                return None; // let legacy handle it
            }
        }
    }

    // Special keys (arrows, F-keys, etc.) have well-established legacy
    // xterm modified formats (\x1b[1;3A for Alt+Up, etc.) that are universally
    // understood. Even Ghostty sends these in legacy format with kitty mode on.
    // Only use CSI u for character keys and keys without legacy representations.
    match key.code {
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Insert
        | KeyCode::Delete
        | KeyCode::F(_) => {
            return None; // let legacy handle these
        }
        _ => {}
    }

    let (codepoint, alternate_shifted) = match key.code {
        KeyCode::Char(c) => {
            let base = canonical_kitty_char(c, mods);
            let shifted = alternate_shifted_codepoint(key, flags);
            (base as u32, shifted)
        }
        KeyCode::Enter => (13, None),
        KeyCode::Tab => (9, None),
        KeyCode::Backspace => (127, None),
        KeyCode::Esc => (27, None),
        _ => return None, // fall back to legacy for unhandled keys
    };

    let modifier = kitty_modifier(mods);
    let event_suffix = kitty_event_suffix(key, flags);

    let sequence = match (alternate_shifted, event_suffix) {
        (Some(shifted), Some(event)) => format!("\x1b[{codepoint}:{shifted};{modifier}:{event}u"),
        (Some(shifted), None) => format!("\x1b[{codepoint}:{shifted};{modifier}u"),
        (None, Some(event)) => format!("\x1b[{codepoint};{modifier}:{event}u"),
        (None, None) => format!("\x1b[{codepoint};{modifier}u"),
    };

    Some(sequence.into_bytes())
}

/// Legacy terminal encoding (standard escape sequences).
fn encode_legacy(key: KeyEvent) -> Vec<u8> {
    let mods = key.modifiers;

    // Modified special keys (arrows, home, end, etc.) use xterm format:
    //   \x1b[1;{modifier}A  for arrows/home/end
    //   \x1b[{n};{modifier}~ for insert/delete/pgup/pgdn
    // The ESC-prefix hack doesn't work for these since they're already escape sequences.
    if !mods.is_empty() {
        if let Some(bytes) = encode_modified_special(key.code, mods) {
            return bytes;
        }
    }

    // Alt modifier on character keys: prefix with ESC
    if mods.contains(KeyModifiers::ALT) {
        let inner = KeyEvent::new(key.code, mods.difference(KeyModifiers::ALT));
        let mut bytes = vec![0x1b];
        bytes.extend(encode_legacy_inner(inner));
        return bytes;
    }
    encode_legacy_inner(key)
}

/// xterm-style encoding for modified special keys.
/// Modifier value: 1 + (shift?1:0) + (alt?2:0) + (ctrl?4:0)
fn encode_modified_special(code: KeyCode, mods: KeyModifiers) -> Option<Vec<u8>> {
    let modifier = xterm_modifier(mods);
    if modifier <= 1 {
        return None; // no modifiers to encode
    }

    match code {
        // CSI 1;{mod}{letter} format
        KeyCode::Up => Some(format!("\x1b[1;{modifier}A").into_bytes()),
        KeyCode::Down => Some(format!("\x1b[1;{modifier}B").into_bytes()),
        KeyCode::Right => Some(format!("\x1b[1;{modifier}C").into_bytes()),
        KeyCode::Left => Some(format!("\x1b[1;{modifier}D").into_bytes()),
        KeyCode::Home => Some(format!("\x1b[1;{modifier}H").into_bytes()),
        KeyCode::End => Some(format!("\x1b[1;{modifier}F").into_bytes()),
        // CSI {n};{mod}~ format
        KeyCode::Insert => Some(format!("\x1b[2;{modifier}~").into_bytes()),
        KeyCode::Delete => Some(format!("\x1b[3;{modifier}~").into_bytes()),
        KeyCode::PageUp => Some(format!("\x1b[5;{modifier}~").into_bytes()),
        KeyCode::PageDown => Some(format!("\x1b[6;{modifier}~").into_bytes()),
        // F1-F4: CSI 1;{mod}{P-S}
        KeyCode::F(1) => Some(format!("\x1b[1;{modifier}P").into_bytes()),
        KeyCode::F(2) => Some(format!("\x1b[1;{modifier}Q").into_bytes()),
        KeyCode::F(3) => Some(format!("\x1b[1;{modifier}R").into_bytes()),
        KeyCode::F(4) => Some(format!("\x1b[1;{modifier}S").into_bytes()),
        // F5-F12: CSI {n};{mod}~
        KeyCode::F(n @ 5..=12) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => unreachable!(),
            };
            Some(format!("\x1b[{code};{modifier}~").into_bytes())
        }
        _ => None,
    }
}

/// xterm modifier encoding: 1 + shift(1) + alt(2) + ctrl(4)
/// Used for legacy modified special keys (arrows, function keys, etc.)
fn xterm_modifier(mods: KeyModifiers) -> u32 {
    let mut m = 1u32;
    if mods.contains(KeyModifiers::SHIFT) {
        m += 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        m += 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        m += 4;
    }
    m
}

/// Kitty protocol modifier encoding: 1 + shift(1) + alt(2) + ctrl(4) + super(8) + hyper(16) + meta(32)
/// Superset of xterm — adds Super/Hyper/Meta bits.
fn kitty_modifier(mods: KeyModifiers) -> u32 {
    let mut m = xterm_modifier(mods);
    if mods.contains(KeyModifiers::SUPER) {
        m += 8;
    }
    if mods.contains(KeyModifiers::HYPER) {
        m += 16;
    }
    if mods.contains(KeyModifiers::META) {
        m += 32;
    }
    m
}

fn canonical_kitty_char(ch: char, mods: KeyModifiers) -> char {
    if mods.contains(KeyModifiers::SHIFT) && ch.is_ascii_uppercase() {
        ch.to_ascii_lowercase()
    } else {
        ch
    }
}

fn alternate_shifted_codepoint(key: &TerminalKey, flags: u16) -> Option<u32> {
    if flags & KITTY_FLAG_REPORT_ALTERNATE_KEYS == 0 {
        return None;
    }

    if let Some(shifted) = key.shifted_codepoint {
        return Some(shifted);
    }

    match key.code {
        KeyCode::Char(ch)
            if key.modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_uppercase() =>
        {
            Some(ch as u32)
        }
        _ => None,
    }
}

fn kitty_event_suffix(key: &TerminalKey, flags: u16) -> Option<u8> {
    if flags & KITTY_FLAG_REPORT_EVENT_TYPES == 0 {
        return None;
    }

    Some(match key.kind {
        crossterm::event::KeyEventKind::Press => 1,
        crossterm::event::KeyEventKind::Repeat => 2,
        crossterm::event::KeyEventKind::Release => 3,
    })
}

fn encode_legacy_inner(key: KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(ch) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let upper = ch.to_ascii_uppercase();
                match upper {
                    'A'..='Z' => vec![upper as u8 - 64],
                    ' ' | '@' | '2' => vec![0],
                    '[' | '3' => vec![27],
                    '\\' | '4' => vec![28],
                    ']' | '5' => vec![29],
                    '^' | '6' => vec![30],
                    '_' | '7' | '-' => vec![31],
                    _ => vec![ch as u8],
                }
            } else {
                let mut buf = [0u8; 4];
                ch.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![9],
        KeyCode::BackTab => vec![27, 91, 90],
        KeyCode::Esc => vec![27],
        KeyCode::Left => vec![27, 91, 68],
        KeyCode::Right => vec![27, 91, 67],
        KeyCode::Up => vec![27, 91, 65],
        KeyCode::Down => vec![27, 91, 66],
        KeyCode::Home => vec![27, 91, 72],
        KeyCode::End => vec![27, 91, 70],
        KeyCode::PageUp => vec![27, 91, 53, 126],
        KeyCode::PageDown => vec![27, 91, 54, 126],
        KeyCode::Delete => vec![27, 91, 51, 126],
        KeyCode::Insert => vec![27, 91, 50, 126],
        KeyCode::F(n) => encode_f_key(n),
        _ => vec![],
    }
}

fn encode_f_key(n: u8) -> Vec<u8> {
    match n {
        1 => vec![27, 79, 80],
        2 => vec![27, 79, 81],
        3 => vec![27, 79, 82],
        4 => vec![27, 79, 83],
        5 => vec![27, 91, 49, 53, 126],
        6 => vec![27, 91, 49, 55, 126],
        7 => vec![27, 91, 49, 56, 126],
        8 => vec![27, 91, 49, 57, 126],
        9 => vec![27, 91, 50, 48, 126],
        10 => vec![27, 91, 50, 49, 126],
        11 => vec![27, 91, 50, 51, 126],
        12 => vec![27, 91, 50, 52, 126],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_terminal_key_eq(
        actual: TerminalKey,
        code: KeyCode,
        modifiers: KeyModifiers,
        kind: crossterm::event::KeyEventKind,
        shifted_codepoint: Option<u32>,
    ) {
        assert_eq!(actual.code, code);
        assert_eq!(actual.modifiers, modifiers);
        assert_eq!(actual.kind, kind);
        assert_eq!(actual.shifted_codepoint, shifted_codepoint);
    }

    fn decode_hex(hex: &str) -> Vec<u8> {
        let hex = hex.trim();
        assert_eq!(hex.len() % 2, 0, "hex string must have even length");
        (0..hex.len())
            .step_by(2)
            .map(|idx| u8::from_str_radix(&hex[idx..idx + 2], 16).unwrap())
            .collect()
    }

    fn parse_fixture_key_code(value: &str) -> KeyCode {
        match value {
            "enter" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "backspace" => KeyCode::Backspace,
            "esc" => KeyCode::Esc,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            "insert" => KeyCode::Insert,
            "delete" => KeyCode::Delete,
            value if value.starts_with("char:") => {
                KeyCode::Char(value.trim_start_matches("char:").chars().next().unwrap())
            }
            other => panic!("unsupported fixture key code: {other}"),
        }
    }

    fn parse_fixture_modifiers(value: &str) -> KeyModifiers {
        if value == "-" || value.is_empty() {
            return KeyModifiers::empty();
        }

        let mut modifiers = KeyModifiers::empty();
        for part in value.split('+') {
            match part {
                "shift" => modifiers |= KeyModifiers::SHIFT,
                "alt" => modifiers |= KeyModifiers::ALT,
                "control" => modifiers |= KeyModifiers::CONTROL,
                "super" => modifiers |= KeyModifiers::SUPER,
                "hyper" => modifiers |= KeyModifiers::HYPER,
                "meta" => modifiers |= KeyModifiers::META,
                other => panic!("unsupported fixture modifier: {other}"),
            }
        }
        modifiers
    }

    fn parse_fixture_kind(value: &str) -> crossterm::event::KeyEventKind {
        match value {
            "press" => crossterm::event::KeyEventKind::Press,
            "repeat" => crossterm::event::KeyEventKind::Repeat,
            "release" => crossterm::event::KeyEventKind::Release,
            other => panic!("unsupported fixture kind: {other}"),
        }
    }

    #[test]
    fn legacy_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), vec![b'\r']);
    }

    #[test]
    fn legacy_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), vec![3]);
    }

    #[test]
    fn legacy_shift_enter_is_just_cr() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        // Enter/Tab/Backspace/Esc aren't special keys with xterm modifier encoding,
        // so Shift+Enter falls through to legacy which just sends CR
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), vec![b'\r']);
    }

    #[test]
    fn legacy_alt_up() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT);
        // xterm modified key format: CSI 1;3A (3 = 1 + Alt)
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[1;3A");
    }

    #[test]
    fn legacy_shift_right() {
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[1;2C");
    }

    #[test]
    fn legacy_ctrl_left() {
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[1;5D");
    }

    #[test]
    fn legacy_ctrl_shift_end() {
        let key = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[1;6F");
    }

    #[test]
    fn legacy_alt_delete() {
        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[3;3~");
    }

    #[test]
    fn legacy_shift_f5() {
        let key = KeyEvent::new(KeyCode::F(5), KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1b[15;2~");
    }

    #[test]
    fn legacy_alt_char_still_esc_prefix() {
        // Alt+a on character keys still uses ESC prefix (not xterm modified)
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"\x1ba");
    }

    #[test]
    fn kitty_shift_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[13;2u");
    }

    #[test]
    fn kitty_ctrl_shift_a() {
        let key = KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[97;6u");
    }

    #[test]
    fn kitty_shift_uppercase_letter_uses_base_codepoint() {
        let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[108;2u");
    }

    #[test]
    fn kitty_shift_uppercase_letter_with_alternate_keys_reports_shifted_codepoint() {
        let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 7 }), b"\x1b[108:76;2:1u");
    }

    #[test]
    fn kitty_shift_lowercase_letter_normalizes_to_base_codepoint() {
        let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[108;2u");
    }

    #[test]
    fn kitty_alt_shift_uppercase_letter_uses_base_codepoint() {
        let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[108;4u");
    }

    #[test]
    fn kitty_ctrl_shift_uppercase_letter_uses_base_codepoint() {
        let key = KeyEvent::new(
            KeyCode::Char('L'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[108;6u");
    }

    #[test]
    fn legacy_shift_uppercase_letter_stays_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Legacy), b"L");
    }

    #[test]
    fn kitty_alt_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[13;3u");
    }

    #[test]
    fn kitty_plain_ctrl_c_uses_legacy() {
        // Plain Ctrl+letter is well-represented in legacy
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), vec![3]);
    }

    #[test]
    fn kitty_unmodified_uses_legacy() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty());
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"a");
    }

    #[test]
    fn kitty_shift_tab() {
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[9;2u");
    }

    #[test]
    fn kitty_ctrl_shift_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 1 }), b"\x1b[13;6u");
    }

    #[test]
    fn kitty_repeat_event_type_is_encoded_when_requested() {
        let key = KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
            crossterm::event::KeyEventKind::Repeat,
        );
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 3 }), b"\x1b[13;2:2u");
    }

    #[test]
    fn kitty_release_event_type_and_alternate_keys_are_encoded_together() {
        let key = KeyEvent::new_with_kind(
            KeyCode::Char('L'),
            KeyModifiers::SHIFT,
            crossterm::event::KeyEventKind::Release,
        );
        assert_eq!(encode_key(key, KeyboardProtocol::Kitty { flags: 7 }), b"\x1b[108:76;2:3u");
    }

    #[test]
    fn kitty_rich_key_data_can_preserve_shifted_symbol_pairs() {
        let key = TerminalKey::new(KeyCode::Char('1'), KeyModifiers::SHIFT)
            .with_shifted_codepoint('!' as u32);
        assert_eq!(
            encode_terminal_key(key, KeyboardProtocol::Kitty { flags: 7 }),
            b"\x1b[49:33;2:1u"
        );
    }

    #[test]
    fn parse_kitty_sequence_preserves_shifted_symbol_pair() {
        let key = parse_terminal_key_sequence("\x1b[49:33;2:1u").unwrap();
        assert_eq!(key.code, KeyCode::Char('1'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
        assert_eq!(key.kind, crossterm::event::KeyEventKind::Press);
        assert_eq!(key.shifted_codepoint, Some('!' as u32));
    }

    #[test]
    fn parse_kitty_sequence_preserves_shifted_letter_pair_and_release() {
        let key = parse_terminal_key_sequence("\x1b[108:76;2:3u").unwrap();
        assert_eq!(key.code, KeyCode::Char('l'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
        assert_eq!(key.kind, crossterm::event::KeyEventKind::Release);
        assert_eq!(key.shifted_codepoint, Some('L' as u32));
    }

    #[test]
    fn parse_modify_other_keys_sequence() {
        let key = parse_terminal_key_sequence("\x1b[27;6;108~").unwrap();
        assert_eq!(key.code, KeyCode::Char('l'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(key.kind, crossterm::event::KeyEventKind::Press);
        assert_eq!(key.shifted_codepoint, None);
    }

    #[test]
    fn parse_legacy_uppercase_letter_as_shifted_char() {
        let key = parse_terminal_key_sequence("L").unwrap();
        assert_eq!(key.code, KeyCode::Char('L'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_legacy_up_arrow_sequence() {
        let key = parse_terminal_key_sequence("\x1b[A").unwrap();
        assert_eq!(key.code, KeyCode::Up);
        assert_eq!(key.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn parse_xterm_alt_up_arrow_sequence() {
        let key = parse_terminal_key_sequence("\x1b[1;3A").unwrap();
        assert_eq!(key.code, KeyCode::Up);
        assert_eq!(key.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn parse_xterm_alt_down_arrow_sequence() {
        let key = parse_terminal_key_sequence("\x1b[1;3B").unwrap();
        assert_eq!(key.code, KeyCode::Down);
        assert_eq!(key.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn parse_kitty_functional_up_arrow_sequence() {
        let key = parse_terminal_key_sequence("\x1b[57419;1u").unwrap();
        assert_eq!(key.code, KeyCode::Up);
        assert_eq!(key.modifiers, KeyModifiers::empty());
    }

    #[test]
    fn parse_legacy_ctrl_b_sequence() {
        let key = parse_terminal_key_sequence("\x02").unwrap();
        assert_eq!(key.code, KeyCode::Char('b'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parse_legacy_ctrl_c_sequence() {
        let key = parse_terminal_key_sequence("\x03").unwrap();
        assert_eq!(key.code, KeyCode::Char('c'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn legacy_ctrl_byte_matrix_is_covered() {
        for (byte, expected) in [
            (b'\x01', 'a'),
            (b'\x02', 'b'),
            (b'\x03', 'c'),
            (b'\x1a', 'z'),
        ] {
            let key = parse_terminal_key_sequence(std::str::from_utf8(&[byte]).unwrap()).unwrap();
            assert_terminal_key_eq(
                key,
                KeyCode::Char(expected),
                KeyModifiers::CONTROL,
                crossterm::event::KeyEventKind::Press,
                None,
            );
        }

        // Ctrl+[ is byte-identical to Escape in legacy terminals, so the parser
        // intentionally treats 0x1b as Escape and only disambiguates the other
        // legacy control-symbol bytes here.
        for (byte, expected) in [
            (b'\x1c', '\\'),
            (b'\x1d', ']'),
            (b'\x1e', '^'),
            (b'\x1f', '-'),
        ] {
            let key = parse_terminal_key_sequence(std::str::from_utf8(&[byte]).unwrap()).unwrap();
            assert_terminal_key_eq(
                key,
                KeyCode::Char(expected),
                KeyModifiers::CONTROL,
                crossterm::event::KeyEventKind::Press,
                None,
            );
        }
    }

    #[test]
    fn legacy_modified_special_roundtrip_matrix() {
        let cases = [
            KeyEvent::new(KeyCode::Up, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Down, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Insert, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT),
        ];

        for key in cases {
            let encoded = encode_key(key, KeyboardProtocol::Legacy);
            let parsed = parse_terminal_key_sequence(std::str::from_utf8(&encoded).unwrap()).unwrap();
            assert_terminal_key_eq(parsed, key.code, key.modifiers, key.kind, None);
        }
    }

    #[test]
    fn kitty_shifted_symbol_roundtrip_preserves_alternate_key() {
        let key = TerminalKey::new(KeyCode::Char('1'), KeyModifiers::SHIFT)
            .with_shifted_codepoint('!' as u32);
        let encoded = encode_terminal_key(key, KeyboardProtocol::Kitty { flags: 7 });
        let parsed = parse_terminal_key_sequence(std::str::from_utf8(&encoded).unwrap()).unwrap();
        assert_terminal_key_eq(
            parsed,
            KeyCode::Char('1'),
            KeyModifiers::SHIFT,
            crossterm::event::KeyEventKind::Press,
            Some('!' as u32),
        );
    }

    #[test]
    fn legacy_basic_special_roundtrip_matrix() {
        let cases = [
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Up, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Left, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Right, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Home, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::End, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Insert, KeyModifiers::empty()),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::empty()),
        ];

        for key in cases {
            let encoded = encode_key(key, KeyboardProtocol::Legacy);
            let parsed = parse_terminal_key_sequence(std::str::from_utf8(&encoded).unwrap()).unwrap();
            assert_terminal_key_eq(parsed, key.code, key.modifiers, key.kind, None);
        }
    }

    #[test]
    fn kitty_functional_key_matrix_is_covered() {
        let cases = [
            ("\x1b[57417;1u", KeyCode::Left),
            ("\x1b[57418;1u", KeyCode::Right),
            ("\x1b[57419;1u", KeyCode::Up),
            ("\x1b[57420;1u", KeyCode::Down),
            ("\x1b[57421;1u", KeyCode::PageUp),
            ("\x1b[57422;1u", KeyCode::PageDown),
            ("\x1b[57423;1u", KeyCode::Home),
            ("\x1b[57424;1u", KeyCode::End),
            ("\x1b[57425;1u", KeyCode::Insert),
            ("\x1b[57426;1u", KeyCode::Delete),
        ];

        for (sequence, code) in cases {
            let parsed = parse_terminal_key_sequence(sequence).unwrap();
            assert_terminal_key_eq(
                parsed,
                code,
                KeyModifiers::empty(),
                crossterm::event::KeyEventKind::Press,
                None,
            );
        }
    }

    #[test]
    fn kitty_shifted_symbol_pair_matrix_is_covered() {
        let cases = [('1', '!'), ('/', '?'), ('[', '{')];

        for (base, shifted) in cases {
            let key = TerminalKey::new(KeyCode::Char(base), KeyModifiers::SHIFT)
                .with_shifted_codepoint(shifted as u32);
            let encoded = encode_terminal_key(key, KeyboardProtocol::Kitty { flags: 7 });
            let parsed = parse_terminal_key_sequence(std::str::from_utf8(&encoded).unwrap()).unwrap();
            assert_terminal_key_eq(
                parsed,
                KeyCode::Char(base),
                KeyModifiers::SHIFT,
                crossterm::event::KeyEventKind::Press,
                Some(shifted as u32),
            );
        }
    }

    fn assert_fixture_corpus_parses(corpus: &str) {
        for line in corpus.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut columns: Vec<_> = line.split('\t').collect();
            if columns.len() == 5 {
                columns.push("");
            }

            let (family, bytes_hex, code, modifiers, kind, shifted) = match columns.len() {
                6 => {
                    if columns[1].chars().all(|ch| ch.is_ascii_hexdigit()) {
                        (
                            columns[0],
                            columns[1],
                            columns[2],
                            columns[3],
                            columns[4],
                            columns[5],
                        )
                    } else {
                        (
                            columns[0],
                            columns[2],
                            columns[3],
                            columns[4],
                            columns[5],
                            "",
                        )
                    }
                }
                7 => (
                    columns[0],
                    columns[2],
                    columns[3],
                    columns[4],
                    columns[5],
                    columns[6],
                ),
                _ => panic!("fixture row must have 6 or 7 columns: {line}"),
            };

            assert!(bytes_hex.chars().all(|ch| ch.is_ascii_hexdigit()), "non-hex fixture bytes for {family}: {bytes_hex}");
            let bytes = decode_hex(bytes_hex);
            let text = std::str::from_utf8(&bytes).unwrap();
            let parsed = parse_terminal_key_sequence(text)
                .unwrap_or_else(|| panic!("fixture failed to parse: {family}"));

            assert_terminal_key_eq(
                parsed,
                parse_fixture_key_code(code),
                parse_fixture_modifiers(modifiers),
                parse_fixture_kind(kind),
                if shifted.is_empty() {
                    None
                } else {
                    Some(shifted.parse::<u32>().unwrap())
                },
            );
        }
    }

    #[test]
    fn keyboard_protocol_corpus_fixture_parses() {
        let corpus = include_str!("../tests/fixtures/keyboard_protocol_corpus.tsv");
        assert_fixture_corpus_parses(corpus);
    }

    #[test]
    fn macos_terminal_variants_fixture_parses() {
        let corpus = include_str!("../tests/fixtures/macos_terminal_variants.tsv");
        for line in corpus.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut columns: Vec<_> = line.split('\t').collect();
            if columns.len() == 6 {
                columns.push("");
            }
            assert_eq!(columns.len(), 7, "macOS fixture row must have 7 columns: {line}");

            let source = format!("{}:{}", columns[0], columns[1]);
            let transformed = [
                source.as_str(),
                columns[2],
                columns[3],
                columns[4],
                columns[5],
                columns[6],
            ]
            .join("\t");
            assert_fixture_corpus_parses(&transformed);
        }
    }

    #[test]
    fn linux_terminal_variants_fixture_parses() {
        let corpus = include_str!("../tests/fixtures/linux_terminal_variants.tsv");
        assert_fixture_corpus_parses(corpus);
    }

    #[test]
    fn protocol_from_zero_flags_is_legacy() {
        assert_eq!(KeyboardProtocol::from_kitty_flags(0), KeyboardProtocol::Legacy);
    }

    #[test]
    fn protocol_from_nonzero_flags_is_kitty() {
        assert_eq!(
            KeyboardProtocol::from_kitty_flags(7),
            KeyboardProtocol::Kitty { flags: 7 }
        );
    }
}
