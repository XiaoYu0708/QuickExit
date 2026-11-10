//! 設定檔（config.toml，放在 exe 同一個資料夾）

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// 只在這個程式在前景時才生效（空字串 = 任何視窗都生效）
    pub target_process: String,
    /// 啟動時自動要求系統管理員權限
    pub run_as_admin: bool,
    pub hotkeys: Hotkeys,
    pub cursor: CursorCfg,
    pub click: ClickCfg,
    pub exit_button: Point,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Hotkeys {
    pub exit: String,
    pub calibrate_exit: String,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CursorMode {
    /// 不按任何鍵（滑鼠本來就看得到）
    None,
    /// 點擊期間一直按住喚出鍵
    Hold,
    /// 先按一下喚出鍵，點完可再按一下收起
    Tap,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CursorCfg {
    pub mode: CursorMode,
    pub key: String,
    pub reveal_delay_ms: u64,
    pub hide_after: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ClickCfg {
    pub move_delay_ms: u64,
    pub hold_ms: u64,
    pub restore_cursor: bool,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_process: String::new(),
            run_as_admin: true,
            hotkeys: Hotkeys::default(),
            cursor: CursorCfg::default(),
            click: ClickCfg::default(),
            exit_button: Point::default(),
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            exit: "F3".into(),
            calibrate_exit: "Shift+F3".into(),
        }
    }
}

impl Default for CursorCfg {
    fn default() -> Self {
        Self {
            mode: CursorMode::Hold,
            key: "LCtrl".into(),
            reveal_delay_ms: 150,
            hide_after: true,
        }
    }
}

impl Default for ClickCfg {
    fn default() -> Self {
        Self {
            move_delay_ms: 60,
            hold_ms: 50,
            restore_cursor: false,
        }
    }
}

impl Default for Point {
    /// 預設值取自 2000x1125 截圖中右上角「退出」圖示的位置
    fn default() -> Self {
        Self { x: 0.9800, y: 0.0340 }
    }
}

impl CursorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CursorMode::None => "none",
            CursorMode::Hold => "hold",
            CursorMode::Tap => "tap",
        }
    }
}

pub fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("config.toml")
}

pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| format!("設定檔格式錯誤：{e}"))
}

/// 讀取設定檔；不存在時建立預設檔。回傳 (設定, 檔案內容)
pub fn load_or_create(path: &Path) -> Result<(Config, String), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok((parse(&text)?, text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let cfg = Config::default();
            let text = save(path, &cfg)?;
            Ok((cfg, text))
        }
        Err(e) => Err(format!("無法讀取設定檔 {}：{e}", path.display())),
    }
}

/// 寫入設定檔（帶註解），回傳寫入的內容
pub fn save(path: &Path, cfg: &Config) -> Result<String, String> {
    let text = render(cfg);
    std::fs::write(path, &text).map_err(|e| format!("無法寫入設定檔 {}：{e}", path.display()))?;
    Ok(text)
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

pub fn render(c: &Config) -> String {
    format!(
        r#"# ================================================================
#  QuickExit 設定檔
#  建議直接在程式視窗裡設定；手動修改請先關閉程式再存檔。
# ================================================================

# 只有當這個程式在前景時快捷鍵才會生效，例如 "game.exe"。
# 留空 = 任何視窗都生效。第一次用校正快捷鍵時會自動填入。
target_process = {target}

# 啟動時自動要求系統管理員權限。
# 遊戲若以系統管理員身分執行，本程式也必須是管理員才能送出按鍵與點擊。
run_as_admin = {admin}

[hotkeys]
# 可用寫法：F1~F24、A~Z、0~9、Numpad0~9、Insert、Delete、Home、End、
#           PageUp、PageDown、Pause、ScrollLock、Mouse3(中鍵)、Mouse4、Mouse5(側鍵)
# 可加修飾鍵：Ctrl+ / Alt+ / Shift+ / Win+   例如 "Ctrl+Alt+Q"
# 建議用單一按鍵（例如 F3 或 Mouse5），避免和遊戲內按住修飾鍵的功能衝突。

# 按下後自動點擊「退出」按鈕
exit = {hk_exit}
# 校正退出按鈕位置：先在遊戲內叫出滑鼠，把滑鼠移到退出按鈕上，再按這個鍵
calibrate_exit = {hk_cal}

[cursor]
# 遊戲中要怎樣叫出滑鼠：
#   "hold" = 點擊期間一直按住喚出鍵（「按住 Ctrl」與「按一下 Ctrl」兩種遊戲設定都適用，推薦）
#   "tap"  = 先按一下喚出鍵再點擊
#   "none" = 不按任何鍵
mode = {mode}
# 喚出滑鼠的按鍵：LCtrl、RCtrl、LAlt、LShift ... 等
key = {ckey}
# 按下喚出鍵後，等多久（毫秒）滑鼠才會出現
reveal_delay_ms = {reveal}
# 只在 mode = "tap" 時有效：點完後再按一次喚出鍵把滑鼠收起來
hide_after = {hide}

[click]
# 滑鼠移到按鈕上後，等多久（毫秒）再按下（讓遊戲偵測到滑鼠懸停）
move_delay_ms = {move_delay}
# 左鍵按住多久（毫秒）
hold_ms = {hold}
# 點完後把滑鼠移回原本位置
restore_cursor = {restore}

# 退出按鈕位置：相對於遊戲畫面的比例（0.0 = 左/上邊緣，1.0 = 右/下邊緣）
# 解析度改變也不用重設。建議用校正快捷鍵自動設定。
[exit_button]
x = {ex:.4}
y = {ey:.4}

"#,
        target = q(&c.target_process),
        admin = c.run_as_admin,
        hk_exit = q(&c.hotkeys.exit),
        hk_cal = q(&c.hotkeys.calibrate_exit),
        mode = q(c.cursor.mode.as_str()),
        ckey = q(&c.cursor.key),
        reveal = c.cursor.reveal_delay_ms,
        hide = c.cursor.hide_after,
        move_delay = c.click.move_delay_ms,
        hold = c.click.hold_ms,
        restore = c.click.restore_cursor,
        ex = c.exit_button.x,
        ey = c.exit_button.y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrip() {
        let c = Config::default();
        assert_eq!(parse(&render(&c)).unwrap(), c);
    }

    #[test]
    fn custom_roundtrip() {
        let mut c = Config::default();
        c.target_process = "遊戲 \"測試\".exe".into();
        c.cursor.mode = CursorMode::Tap;
        c.exit_button = Point { x: 0.1234, y: 0.5678 };
        assert_eq!(parse(&render(&c)).unwrap(), c);
    }

    #[test]
    fn old_v010_config_still_loads() {
        let c = parse("[hotkeys]
calibrate_confirm = \"Shift+F10\"
[confirm_button]
enabled = true
x = 0.5
").unwrap();
        assert_eq!(c.hotkeys, Hotkeys::default());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let c = parse("[hotkeys]\nexit = \"Mouse5\"\n").unwrap();
        assert_eq!(c.hotkeys.exit, "Mouse5");
        assert_eq!(c.hotkeys.calibrate_exit, Hotkeys::default().calibrate_exit);
        assert_eq!(c.cursor, CursorCfg::default());
    }
}
