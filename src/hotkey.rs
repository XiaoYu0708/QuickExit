//! 快捷鍵字串解析，例如 "F9"、"Shift+F9"、"Ctrl+Alt+Q"、"Mouse4"。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12; // Alt
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

/// 修飾鍵類別（用來判斷「觸發鍵本身就是修飾鍵」的情況）
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModKind {
    Ctrl,
    Alt,
    Shift,
    Win,
}

fn mod_kind(vk: u16) -> Option<ModKind> {
    match vk {
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(ModKind::Ctrl),
        VK_MENU | VK_LMENU | VK_RMENU => Some(ModKind::Alt),
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(ModKind::Shift),
        VK_LWIN | VK_RWIN => Some(ModKind::Win),
        _ => None,
    }
}

/// 解析單一按鍵名稱（不含修飾鍵組合）
pub fn parse_key(name: &str) -> Option<Trigger> {
    let n = name.trim().to_ascii_lowercase();
    let k = |vk: u16| Some(Trigger::Key(vk));

    // F1 ~ F24
    if let Some(num) = n.strip_prefix('f') {
        if let Ok(i) = num.parse::<u16>() {
            if (1..=24).contains(&i) {
                return k(0x70 + i - 1);
            }
        }
    }
    // Numpad0 ~ Numpad9
    for prefix in ["numpad", "num"] {
        if let Some(num) = n.strip_prefix(prefix) {
            if let Ok(i) = num.parse::<u16>() {
                if i <= 9 {
                    return k(0x60 + i);
                }
            }
        }
    }
    // 單一字母或數字
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return k(c.to_ascii_uppercase() as u16);
        }
        if c.is_ascii_digit() {
            return k(c as u16);
        }
    }

    match n.as_str() {
        "ctrl" | "control" => k(VK_CONTROL),
        "lctrl" | "lcontrol" => k(VK_LCONTROL),
        "rctrl" | "rcontrol" => k(VK_RCONTROL),
        "alt" => k(VK_MENU),
        "lalt" => k(VK_LMENU),
        "ralt" => k(VK_RMENU),
        "shift" => k(VK_SHIFT),
        "lshift" => k(VK_LSHIFT),
        "rshift" => k(VK_RSHIFT),
        "win" | "lwin" => k(VK_LWIN),
        "rwin" => k(VK_RWIN),

        "space" => k(0x20),
        "tab" => k(0x09),
        "enter" | "return" => k(0x0D),
        "esc" | "escape" => k(0x1B),
        "backspace" => k(0x08),
        "insert" | "ins" => k(0x2D),
        "delete" | "del" => k(0x2E),
        "home" => k(0x24),
        "end" => k(0x23),
        "pageup" | "pgup" => k(0x21),
        "pagedown" | "pgdn" => k(0x22),
        "left" => k(0x25),
        "up" => k(0x26),
        "right" => k(0x27),
        "down" => k(0x28),
        "pause" => k(0x13),
        "scrolllock" => k(0x91),
        "capslock" => k(0x14),
        "printscreen" => k(0x2C),
        "numlock" => k(0x90),
        "multiply" | "nummultiply" => k(0x6A),
        "add" | "numadd" => k(0x6B),
        "subtract" | "numsubtract" => k(0x6D),
        "decimal" | "numdecimal" => k(0x6E),
        "divide" | "numdivide" => k(0x6F),
        ";" | "semicolon" => k(0xBA),
        "=" | "equals" => k(0xBB),
        "," | "comma" => k(0xBC),
        "-" | "minus" => k(0xBD),
        "." | "period" => k(0xBE),
        "/" | "slash" => k(0xBF),
        "`" | "grave" | "backtick" => k(0xC0),
        "[" | "lbracket" => k(0xDB),
        "\\" | "backslash" => k(0xDC),
        "]" | "rbracket" => k(0xDD),
        "'" | "quote" => k(0xDE),

        "mbutton" | "middle" | "mouse3" => Some(Trigger::MouseMiddle),
        "xbutton1" | "mouse4" => Some(Trigger::MouseX1),
        "xbutton2" | "mouse5" => Some(Trigger::MouseX2),
        _ => None,
    }
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

impl Hotkey {
    /// 觸發鍵是否與輸入事件相符
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

    /// 目前實體按住的修飾鍵是否與設定完全相同
    pub fn modifiers_match(&self) -> bool {
        let own = match self.trigger {
            Trigger::Key(vk) => mod_kind(vk),
            _ => None,
        };
        let check = |kind: ModKind, want: bool, actual: bool| own == Some(kind) || want == actual;
        check(ModKind::Ctrl, self.ctrl, is_down(VK_CONTROL))
            && check(ModKind::Alt, self.alt, is_down(VK_MENU))
            && check(ModKind::Shift, self.shift, is_down(VK_SHIFT))
            && check(ModKind::Win, self.win, is_down(VK_LWIN) || is_down(VK_RWIN))
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
    fn generic_modifier_matches_sides() {
        let h = parse_hotkey("Ctrl").unwrap();
        assert!(h.trigger_matches(Trigger::Key(VK_LCONTROL)));
        assert!(h.trigger_matches(Trigger::Key(VK_RCONTROL)));
        assert!(!h.trigger_matches(Trigger::Key(VK_LSHIFT)));
    }
}
