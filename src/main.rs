//! QuickExit：按自訂快捷鍵，自動喚出滑鼠並點擊遊戲右上角的「退出」按鈕。

mod config;
mod hotkey;
mod win;

use std::ptr::null_mut;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock, RwLock};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, LRESULT, SYSTEMTIME, WPARAM};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, SetConsoleTitleW, STD_INPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW,
    TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MB_ICONHAND,
    MB_OK, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use config::{Config, CursorMode};
use hotkey::{Hotkey, Trigger};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------- 全域狀態

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Exit,
    CalibrateExit,
    CalibrateConfirm,
}

struct Bindings {
    list: Vec<(Hotkey, Action)>,
    target: String,
}

struct Runtime {
    cfg: Config,
    cursor_vk: u16,
}

static BINDINGS: RwLock<Bindings> = RwLock::new(Bindings { list: Vec::new(), target: String::new() });
static RUNTIME: RwLock<Option<Runtime>> = RwLock::new(None);
/// 最後一次讀取 / 寫入的設定檔內容，用來偵測檔案是否被修改
static FILE_TEXT: Mutex<String> = Mutex::new(String::new());
static TX: OnceLock<Sender<Action>> = OnceLock::new();
/// 已觸發而尚未放開的按鍵（避免長按連發，並一併攔截放開事件）
static DOWN: Mutex<[bool; 260]> = Mutex::new([false; 260]);

fn now() -> String {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
}

macro_rules! log {
    ($($t:tt)*) => { println!("[{}] {}", now(), format!($($t)*)) };
}

fn beep(ok: bool) {
    unsafe { MessageBeep(if ok { MB_OK } else { MB_ICONHAND }) };
}

// ---------------------------------------------------------------- 設定套用

fn apply_config(cfg: Config) -> Result<(), String> {
    let pairs = [
        ("hotkeys.exit", &cfg.hotkeys.exit, Action::Exit),
        ("hotkeys.calibrate_exit", &cfg.hotkeys.calibrate_exit, Action::CalibrateExit),
        ("hotkeys.calibrate_confirm", &cfg.hotkeys.calibrate_confirm, Action::CalibrateConfirm),
    ];
    let mut list: Vec<(Hotkey, Action)> = Vec::new();
    for (name, text, action) in pairs {
        if text.trim().is_empty() {
            continue; // 留空 = 停用此快捷鍵
        }
        let hk = hotkey::parse_hotkey(text).map_err(|e| format!("{name}：{e}"))?;
        if let Some((other, _)) = list.iter().find(|(o, _)| {
            o.trigger == hk.trigger && (o.ctrl, o.alt, o.shift, o.win) == (hk.ctrl, hk.alt, hk.shift, hk.win)
        }) {
            return Err(format!("{name} 的快捷鍵 \"{}\" 與 \"{}\" 重複", hk.text, other.text));
        }
        list.push((hk, action));
    }
    if !list.iter().any(|(_, a)| *a == Action::Exit) {
        return Err("hotkeys.exit 不可為空".into());
    }

    let cursor_vk = match hotkey::parse_key(&cfg.cursor.key) {
        Some(Trigger::Key(vk)) => vk,
        _ => return Err(format!("cursor.key：看不懂的按鍵名稱 \"{}\"", cfg.cursor.key)),
    };

    *BINDINGS.write().unwrap() = Bindings { list, target: cfg.target_process.trim().to_string() };
    *RUNTIME.write().unwrap() = Some(Runtime { cfg, cursor_vk });
    Ok(())
}

fn print_summary() {
    let guard = RUNTIME.read().unwrap();
    let Some(rt) = guard.as_ref() else { return };
    let c = &rt.cfg;
    let mode = match c.cursor.mode {
        CursorMode::Hold => format!("按住 {}", c.cursor.key),
        CursorMode::Tap => format!("按一下 {}", c.cursor.key),
        CursorMode::None => "不按鍵".to_string(),
    };
    println!("  退出快捷鍵       : {}", c.hotkeys.exit);
    println!("  校正退出按鈕     : {}", c.hotkeys.calibrate_exit);
    println!("  校正確認按鈕     : {}", c.hotkeys.calibrate_confirm);
    println!("  喚出滑鼠方式     : {mode}");
    println!(
        "  目標程式         : {}",
        if c.target_process.is_empty() { "（任何視窗）" } else { &c.target_process }
    );
    println!("  退出按鈕位置     : x={:.4}, y={:.4}", c.exit_button.x, c.exit_button.y);
    if c.confirm_button.enabled {
        println!(
            "  確認按鈕位置     : x={:.4}, y={:.4}（{} ms 後點擊）",
            c.confirm_button.x, c.confirm_button.y, c.confirm_button.delay_ms
        );
    } else {
        println!("  確認按鈕         : 未啟用");
    }
}

// ---------------------------------------------------------------- 鍵盤 / 滑鼠 Hook

fn input_index(t: Trigger) -> usize {
    match t {
        Trigger::Key(vk) => (vk as usize) & 0xFF,
        Trigger::MouseMiddle => 256,
        Trigger::MouseX1 => 257,
        Trigger::MouseX2 => 258,
    }
}

fn target_matches(target: &str) -> bool {
    if target.is_empty() {
        return true;
    }
    win::foreground_process_name()
        .map(|n| n.to_lowercase() == target.to_lowercase())
        .unwrap_or(false)
}

/// 回傳 true 表示攔截此事件（不傳給遊戲）
fn handle_input(input: Trigger, down: bool, up: bool) -> bool {
    let idx = input_index(input);
    let mut pressed = DOWN.lock().unwrap();
    if up {
        return std::mem::replace(&mut pressed[idx], false);
    }
    if !down {
        return false;
    }
    if pressed[idx] {
        return true; // 長按的重複事件
    }

    let b = BINDINGS.read().unwrap();
    let hit = b
        .list
        .iter()
        .find(|(hk, _)| hk.trigger_matches(input) && hk.modifiers_match())
        .map(|(_, a)| *a);
    let Some(action) = hit else { return false };
    if !target_matches(&b.target) {
        return false;
    }
    pressed[idx] = true;
    if let Some(tx) = TX.get() {
        let _ = tx.send(action);
    }
    true
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        if kb.flags & LLKHF_INJECTED == 0 {
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if handle_input(Trigger::Key(kb.vkCode as u16), down, up) {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let ms = &*(lparam as *const MSLLHOOKSTRUCT);
        if ms.flags & LLMHF_INJECTED == 0 {
            let msg = wparam as u32;
            let xbtn = || if (ms.mouseData >> 16) & 0xFFFF == 1 { Trigger::MouseX1 } else { Trigger::MouseX2 };
            let event = match msg {
                WM_MBUTTONDOWN => Some((Trigger::MouseMiddle, true)),
                WM_MBUTTONUP => Some((Trigger::MouseMiddle, false)),
                WM_XBUTTONDOWN => Some((xbtn(), true)),
                WM_XBUTTONUP => Some((xbtn(), false)),
                _ => None,
            };
            if let Some((t, is_down)) = event {
                if handle_input(t, is_down, !is_down) {
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

// ---------------------------------------------------------------- 動作

fn click_ratio(area: &win::ClientArea, x: f64, y: f64, c: &Config) {
    let (sx, sy) = area.ratio_to_screen(x, y);
    win::move_mouse(sx, sy);
    win::sleep_ms(c.click.move_delay_ms);
    win::left_click(c.click.hold_ms);
}

fn do_exit() {
    let (cfg, cursor_vk) = {
        let g = RUNTIME.read().unwrap();
        let rt = g.as_ref().unwrap();
        (rt.cfg.clone(), rt.cursor_vk)
    };
    let Some(area) = win::foreground_client() else {
        log!("失敗：找不到前景視窗");
        beep(false);
        return;
    };

    // 等使用者放開 Alt / Shift / Win，避免變成「Alt+點擊」之類的組合（最多 1 秒）
    for _ in 0..50 {
        let held = [hotkey::VK_MENU, hotkey::VK_SHIFT, hotkey::VK_LWIN, hotkey::VK_RWIN]
            .iter()
            .any(|&vk| win::key_is_down(vk));
        if !held {
            break;
        }
        win::sleep_ms(20);
    }

    let original = win::cursor_pos();

    match cfg.cursor.mode {
        CursorMode::Hold => win::send_key(cursor_vk, false),
        CursorMode::Tap => win::tap_key(cursor_vk),
        CursorMode::None => {}
    }
    if cfg.cursor.mode != CursorMode::None {
        win::sleep_ms(cfg.cursor.reveal_delay_ms);
    }

    click_ratio(&area, cfg.exit_button.x, cfg.exit_button.y, &cfg);
    if cfg.confirm_button.enabled {
        win::sleep_ms(cfg.confirm_button.delay_ms);
        click_ratio(&area, cfg.confirm_button.x, cfg.confirm_button.y, &cfg);
    }

    match cfg.cursor.mode {
        CursorMode::Hold => win::send_key(cursor_vk, true),
        CursorMode::Tap if cfg.cursor.hide_after => {
            win::sleep_ms(80);
            win::tap_key(cursor_vk);
        }
        _ => {}
    }

    if cfg.click.restore_cursor {
        if let Some((x, y)) = original {
            win::move_mouse(x, y);
        }
    }

    let (sx, sy) = area.ratio_to_screen(cfg.exit_button.x, cfg.exit_button.y);
    log!(
        "已點擊退出按鈕（螢幕座標 {sx},{sy}，視窗 {}x{}）{}",
        area.width,
        area.height,
        if cfg.confirm_button.enabled { "＋確認按鈕" } else { "" }
    );
}

fn do_calibrate(confirm: bool) {
    let label = if confirm { "確認按鈕" } else { "退出按鈕" };
    let (Some(area), Some((x, y))) = (win::foreground_client(), win::cursor_pos()) else {
        log!("校正{label}失敗：找不到前景視窗或滑鼠位置");
        beep(false);
        return;
    };
    let (rx, ry) = area.screen_to_ratio(x, y);
    if !(0.0..=1.0).contains(&rx) || !(0.0..=1.0).contains(&ry) {
        log!("校正{label}失敗：滑鼠不在前景視窗範圍內");
        beep(false);
        return;
    }

    let mut cfg = RUNTIME.read().unwrap().as_ref().unwrap().cfg.clone();
    if confirm {
        cfg.confirm_button.x = rx;
        cfg.confirm_button.y = ry;
        cfg.confirm_button.enabled = true;
    } else {
        cfg.exit_button.x = rx;
        cfg.exit_button.y = ry;
    }

    let mut bound = None;
    if cfg.target_process.is_empty() {
        let own = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        if let Some(name) = win::process_name(area.hwnd) {
            if Some(name.to_lowercase()) != own {
                cfg.target_process = name.clone();
                bound = Some(name);
            }
        }
    }

    let path = config::config_path();
    match config::save(&path, &cfg) {
        Ok(text) => {
            *FILE_TEXT.lock().unwrap() = text;
            if let Err(e) = apply_config(cfg) {
                log!("套用設定失敗：{e}");
                beep(false);
                return;
            }
            log!("已校正{label}：x={rx:.4}, y={ry:.4}（視窗 {}x{}）", area.width, area.height);
            if let Some(name) = bound {
                log!("已自動綁定目標程式：{name}（之後快捷鍵只在這個程式中生效）");
            }
            beep(true);
        }
        Err(e) => {
            log!("校正{label}失敗：{e}");
            beep(false);
        }
    }
}

// ---------------------------------------------------------------- 背景執行緒

fn spawn_worker() {
    let (tx, rx) = channel::<Action>();
    TX.set(tx).ok();
    std::thread::spawn(move || {
        for action in rx {
            match action {
                Action::Exit => do_exit(),
                Action::CalibrateExit => do_calibrate(false),
                Action::CalibrateConfirm => do_calibrate(true),
            }
        }
    });
}

/// 每秒檢查設定檔是否被修改，有改就重新載入
fn spawn_config_watcher() {
    std::thread::spawn(|| {
        let path = config::config_path();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            {
                let mut last = FILE_TEXT.lock().unwrap();
                if *last == text {
                    continue;
                }
                *last = text.clone();
            }
            match config::parse(&text).and_then(apply_config) {
                Ok(()) => {
                    log!("設定檔已重新載入");
                    print_summary();
                }
                Err(e) => log!("設定檔有錯誤，繼續使用舊設定：{e}"),
            }
        }
    });
}

// ---------------------------------------------------------------- 啟動

fn pause() {
    println!("\n按 Enter 結束...");
    let _ = std::io::stdin().read_line(&mut String::new());
}

fn setup_console() {
    unsafe {
        let title = win::wide(&format!("QuickExit v{VERSION} - 副本退出快捷鍵"));
        SetConsoleTitleW(title.as_ptr());
        // 關閉「快速編輯模式」，避免點到主控台視窗時整個程式被暫停
        let h = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0u32;
        if GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleMode(h, (mode & !0x0040) | 0x0080); // ~ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS
        }
    }
}

fn main() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    setup_console();

    println!("==============================================");
    println!("  QuickExit v{VERSION}  副本退出快捷鍵");
    println!("==============================================");

    let path = config::config_path();
    let (cfg, text) = match config::load_or_create(&path) {
        Ok(v) => v,
        Err(e) => {
            println!("錯誤：{e}");
            pause();
            return;
        }
    };

    if cfg.run_as_admin && !win::is_admin() {
        println!("正在要求系統管理員權限...");
        if win::relaunch_as_admin() {
            return;
        }
        println!("警告：未取得系統管理員權限。若遊戲以管理員身分執行，快捷鍵可能無效。\n");
    }

    let mutex_name = win::wide("Local\\QuickExit_SingleInstance");
    unsafe {
        CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            println!("QuickExit 已經在執行中了。");
            pause();
            return;
        }
    }

    *FILE_TEXT.lock().unwrap() = text;
    if let Err(e) = apply_config(cfg) {
        println!("設定檔錯誤：{e}\n請修改 {} 後重新開啟。", path.display());
        pause();
        return;
    }

    println!("設定檔：{}", path.display());
    println!("權限  ：{}", if win::is_admin() { "系統管理員" } else { "一般使用者" });
    println!();
    print_summary();
    println!();
    println!("第一次使用請先校正：在遊戲中叫出滑鼠、移到右上角退出按鈕上，");
    println!("再按「校正退出按鈕」快捷鍵，聽到提示音就完成了。");
    println!("關閉這個視窗即可結束程式。");
    println!("----------------------------------------------");

    spawn_worker();
    spawn_config_watcher();

    unsafe {
        let hmod = GetModuleHandleW(std::ptr::null());
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
        if kb.is_null() || ms.is_null() {
            println!("錯誤：無法安裝鍵盤/滑鼠監聽（錯誤碼 {}）", GetLastError());
            pause();
            return;
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
