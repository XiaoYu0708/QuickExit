//! 快捷鍵字串解析與產生，例如 "F9"、"Shift+F9"、"Ctrl+Alt+Q"、"Mouse5"。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12; // Alt
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(u16),
    MouseMiddle,
    MouseX1,
    MouseX2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub trigger: Trigger,
    pub text: String,
}

/// 具名按鍵表：同一個鍵碼的第一個名稱是「正式名稱」，其餘是別名
const NAMED_KEYS: &[(&str, u16)] = &[
    ("Ctrl", VK_CONTROL),
    ("Control", VK_CONTROL),
    ("LCtrl", VK_LCONTROL),
    ("LControl", VK_LCONTROL),
    ("RCtrl", VK_RCONTROL),
    ("RControl", VK_RCONTROL),
    ("Alt", VK_MENU),
    ("LAlt", VK_LMENU),
    ("RAlt", VK_RMENU),
    ("Shift", VK_SHIFT),
    ("LShift", VK_LSHIFT),
    ("RShift", VK_RSHIFT),
    ("LWin", VK_LWIN),
    ("Win", VK_LWIN),
    ("RWin", VK_RWIN),
    ("Space", 0x20),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Return", 0x0D),
    ("Esc", VK_ESCAPE),
    ("Escape", VK_ESCAPE),
    ("Backspace", 0x08),
    ("Insert", 0x2D),
    ("Ins", 0x2D),
    ("Delete", 0x2E),
    ("Del", 0x2E),
    ("Home", 0x24),
    ("End", 0x23),
    ("PageUp", 0x21),
    ("PgUp", 0x21),
    ("PageDown", 0x22),
    ("PgDn", 0x22),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Pause", 0x13),
    ("ScrollLock", 0x91),
    ("CapsLock", 0x14),
    ("PrintScreen", 0x2C),
    ("NumLock", 0x90),
    ("NumMultiply", 0x6A),
    ("Multiply", 0x6A),
    ("NumAdd", 0x6B),
    ("Add", 0x6B),
    ("NumSubtract", 0x6D),
    ("Subtract", 0x6D),
    ("NumDecimal", 0x6E),
    ("Decimal", 0x6E),
    ("NumDivide", 0x6F),
    ("Divide", 0x6F),
    (";", 0xBA),
    ("Semicolon", 0xBA),
    ("=", 0xBB),
    ("Equals", 0xBB),
    (",", 0xBC),
    ("Comma", 0xBC),
    ("-", 0xBD),
    ("Minus", 0xBD),
    (".", 0xBE),
    ("Period", 0xBE),
    ("/", 0xBF),
    ("Slash", 0xBF),
    ("`", 0xC0),
    ("Grave", 0xC0),
    ("Backtick", 0xC0),
    ("[", 0xDB),
    ("LBracket", 0xDB),
    ("\\", 0xDC),
    ("Backslash", 0xDC),
    ("]", 0xDD),
    ("RBracket", 0xDD),
    ("'", 0xDE),
    ("Quote", 0xDE),
];

const NAMED_MOUSE: &[(&str, Trigger)] = &[
    ("Mouse3", Trigger::MouseMiddle),
    ("MButton", Trigger::MouseMiddle),
    ("Middle", Trigger::MouseMiddle),
    ("Mouse4", Trigger::MouseX1),
    ("XButton1", Trigger::MouseX1),
    ("Mouse5", Trigger::MouseX2),
    ("XButton2", Trigger::MouseX2),
];

/// 解析單一按鍵名稱（不含修飾鍵組合）
pub fn parse_key(name: &str) -> Option<Trigger> {
    let n = name.trim().to_ascii_lowercase();

    if let Some(i) = n.strip_prefix('f').and_then(|s| s.parse::<u16>().ok()) {
        if (1..=24).contains(&i) {
            return Some(Trigger::Key(0x70 + i - 1));
        }
    }
    for prefix in ["numpad", "num"] {
        if let Some(i) = n.strip_prefix(prefix).and_then(|s| s.parse::<u16>().ok()) {
            if i <= 9 {
                return Some(Trigger::Key(0x60 + i));
            }
        }
    }
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(Trigger::Key(c.to_ascii_uppercase() as u16));
        }
        if c.is_ascii_digit() {
            return Some(Trigger::Key(c as u16));
        }
    }
    NAMED_KEYS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&n))
        .map(|&(_, vk)| Trigger::Key(vk))
        .or_else(|| NAMED_MOUSE.iter().find(|(k, _)| k.eq_ignore_ascii_case(&n)).map(|&(_, t)| t))
}

/// 按鍵的正式名稱（parse_key 的反向）
pub fn key_name(t: Trigger) -> Option<String> {
    let vk = match t {
        Trigger::Key(vk) => vk,
        mouse => return NAMED_MOUSE.iter().find(|(_, m)| *m == mouse).map(|(n, _)| n.to_string()),
    };
    match vk {
        0x70..=0x87 => Some(format!("F{}", vk - 0x70 + 1)),
        0x60..=0x69 => Some(format!("Numpad{}", vk - 0x60)),
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        _ => NAMED_KEYS.iter().find(|(_, v)| *v == vk).map(|(n, _)| n.to_string()),
    }
}

pub fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU | VK_SHIFT | VK_LSHIFT
            | VK_RSHIFT | VK_LWIN | VK_RWIN
    )
}

/// 解析快捷鍵組合，例如 "Ctrl+Alt+F9"
pub fn parse_hotkey(s: &str) -> Result<Hotkey, String> {
    let text = s.trim().to_string();
    if text.is_empty() {
        return Err("快捷鍵不可為空".into());
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("快捷鍵格式錯誤：\"{text}\""));
    }
    let (last, mods) = parts.split_last().unwrap();
    let mut hk = Hotkey {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
        trigger: parse_key(last).ok_or_else(|| format!("看不懂的按鍵名稱：\"{last}\""))?,
        text: text.clone(),
    };
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => hk.ctrl = true,
            "alt" => hk.alt = true,
            "shift" => hk.shift = true,
            "win" => hk.win = true,
            other => return Err(format!("\"{other}\" 不是修飾鍵（只能用 Ctrl / Alt / Shift / Win）")),
        }
    }
    Ok(hk)
}

fn is_down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// 目前實體按住的修飾鍵 (ctrl, alt, shift, win)
pub fn current_modifiers() -> (bool, bool, bool, bool) {
    (
        is_down(VK_CONTROL),
        is_down(VK_MENU),
        is_down(VK_SHIFT),
        is_down(VK_LWIN) || is_down(VK_RWIN),
    )
}

/// 依目前按住的修飾鍵＋觸發鍵組出快捷鍵字串（錄製快捷鍵用）
pub fn format_combo(mods: (bool, bool, bool, bool), t: Trigger) -> Option<String> {
    let mut s = String::new();
    for (on, name) in [(mods.0, "Ctrl+"), (mods.1, "Alt+"), (mods.2, "Shift+"), (mods.3, "Win+")] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(&key_name(t)?);
    Some(s)
}

impl Hotkey {
    pub fn trigger_matches(&self, input: Trigger) -> bool {
        match (self.trigger, input) {
            (Trigger::Key(want), Trigger::Key(got)) => {
                want == got
                    || match want {
                        VK_CONTROL => got == VK_LCONTROL || got == VK_RCONTROL,
                        VK_MENU => got == VK_LMENU || got == VK_RMENU,
                        VK_SHIFT => got == VK_LSHIFT || got == VK_RSHIFT,
                        VK_LWIN => got == VK_RWIN,
                        _ => false,
                    }
            }
            (a, b) => a == b,
        }
    }

    pub fn same_combo(&self, other: &Hotkey) -> bool {
        self.trigger == other.trigger
            && (self.ctrl, self.alt, self.shift, self.win) == (other.ctrl, other.alt, other.shift, other.win)
    }

    /// 目前實體按住的修飾鍵是否與設定完全相同
    pub fn modifiers_match(&self) -> bool {
        let (c, a, s, w) = current_modifiers();
        (self.ctrl, self.alt, self.shift, self.win) == (c, a, s, w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let h = parse_hotkey("F9").unwrap();
        assert_eq!(h.trigger, Trigger::Key(0x78));
        assert!(!h.ctrl && !h.alt && !h.shift && !h.win);
    }

    #[test]
    fn parse_combo() {
        let h = parse_hotkey(" ctrl + Alt+q ").unwrap();
        assert!(h.ctrl && h.alt && !h.shift);
        assert_eq!(h.trigger, Trigger::Key(b'Q' as u16));
    }

    #[test]
    fn parse_mouse_and_numpad() {
        assert_eq!(parse_hotkey("Mouse5").unwrap().trigger, Trigger::MouseX2);
        assert_eq!(parse_hotkey("Numpad0").unwrap().trigger, Trigger::Key(0x60));
        assert_eq!(parse_hotkey("F24").unwrap().trigger, Trigger::Key(0x87));
    }

    #[test]
    fn parse_errors() {
        assert!(parse_hotkey("").is_err());
        assert!(parse_hotkey("Ctrl+").is_err());
        assert!(parse_hotkey("Foo").is_err());
        assert!(parse_hotkey("Q+F9").is_err());
        assert!(parse_hotkey("F25").is_err());
    }

    #[test]
    fn key_name_roundtrip() {
        for vk in 0u16..=0xFF {
            if let Some(name) = key_name(Trigger::Key(vk)) {
                assert_eq!(parse_key(&name), Some(Trigger::Key(vk)), "vk {vk:#x} -> {name}");
            }
        }
        for t in [Trigger::MouseMiddle, Trigger::MouseX1, Trigger::MouseX2] {
            assert_eq!(parse_key(&key_name(t).unwrap()), Some(t));
        }
    }

    #[test]
    fn format_combo_parses_back() {
        let s = format_combo((true, false, true, false), Trigger::Key(0x78)).unwrap();
        assert_eq!(s, "Ctrl+Shift+F9");
        let h = parse_hotkey(&s).unwrap();
        assert!(h.ctrl && h.shift && !h.alt && !h.win);
    }
}
