//! 背景引擎：全域鍵盤/滑鼠 Hook、快捷鍵錄製、自動點擊、校正、設定儲存、事件紀錄。
//! GUI 只透過這裡的公開函式讀寫狀態。

use std::collections::VecDeque;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock, RwLock};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, SYSTEMTIME, WPARAM};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION,
    KBDLLHOOKSTRUCT, MB_ICONHAND, MB_OK, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::config::{self, Config, CursorMode};
use crate::hotkey::{self, Hotkey, Trigger};
use crate::win;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Exit,
    Calibrate,
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
/// 設定被引擎改動（例如校正）時遞增，GUI 據此重新讀取
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
static TX: OnceLock<Sender<Action>> = OnceLock::new();
/// 已觸發而尚未放開的按鍵（避免長按連發，並一併攔截放開事件）
static DOWN: Mutex<[bool; 260]> = Mutex::new([false; 260]);
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static REPAINT: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// 快捷鍵錄製：CAPTURING 為 true 時，下一個按鍵會被記錄到 CAPTURED
static CAPTURING: AtomicBool = AtomicBool::new(false);
/// Some(Some(s)) = 錄到 s；Some(None) = 使用者按 Esc 取消
static CAPTURED: Mutex<Option<Option<String>>> = Mutex::new(None);

// ---------------------------------------------------------------- 紀錄 / 通知

fn now() -> String {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
}

pub fn log(msg: impl Into<String>) {
    let mut l = LOG.lock().unwrap();
    l.push_back(format!("[{}] {}", now(), msg.into()));
    while l.len() > 200 {
        l.pop_front();
    }
    drop(l);
    repaint();
}

pub fn logs() -> Vec<String> {
    LOG.lock().unwrap().iter().cloned().collect()
}

pub fn clear_logs() {
    LOG.lock().unwrap().clear();
}

fn repaint() {
    if let Some(f) = REPAINT.get() {
        f();
    }
}

/// GUI 啟動後註冊重繪函式，讓背景執行緒有事件時能更新畫面
pub fn set_repaint(f: impl Fn() + Send + Sync + 'static) {
    let _ = REPAINT.set(Box::new(f));
}

fn beep(ok: bool) {
    unsafe { MessageBeep(if ok { MB_OK } else { MB_ICONHAND }) };
}

// ---------------------------------------------------------------- 設定

fn validate(cfg: &Config) -> Result<(Vec<(Hotkey, Action)>, u16), String> {
    let exit = hotkey::parse_hotkey(&cfg.hotkeys.exit).map_err(|e| format!("退出快捷鍵：{e}"))?;
    let mut list = vec![(exit, Action::Exit)];
    if !cfg.hotkeys.calibrate_exit.trim().is_empty() {
        let cal = hotkey::parse_hotkey(&cfg.hotkeys.calibrate_exit).map_err(|e| format!("校正快捷鍵：{e}"))?;
        if cal.same_combo(&list[0].0) {
            return Err("退出快捷鍵和校正快捷鍵不能相同".into());
        }
        list.push((cal, Action::Calibrate));
    }
    let cursor_vk = match hotkey::parse_key(&cfg.cursor.key) {
        Some(Trigger::Key(vk)) => vk,
        _ => return Err(format!("喚出鍵：看不懂的按鍵名稱 \"{}\"", cfg.cursor.key)),
    };
    Ok((list, cursor_vk))
}

fn apply(cfg: Config) -> Result<(), String> {
    let (list, cursor_vk) = validate(&cfg)?;
    *BINDINGS.write().unwrap() = Bindings { list, target: cfg.target_process.trim().to_string() };
    *RUNTIME.write().unwrap() = Some(Runtime { cfg, cursor_vk });
    Ok(())
}

/// 套用並儲存設定（GUI 修改時呼叫）
pub fn update_config(cfg: Config) -> Result<(), String> {
    validate(&cfg)?;
    config::save(&config::config_path(), &cfg)?;
    apply(cfg)
}

pub fn current_config() -> Config {
    RUNTIME.read().unwrap().as_ref().map(|r| r.cfg.clone()).unwrap_or_default()
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------- 快捷鍵錄製

pub fn start_capture() {
    *CAPTURED.lock().unwrap() = None;
    CAPTURING.store(true, Ordering::SeqCst);
}

pub fn cancel_capture() {
    CAPTURING.store(false, Ordering::SeqCst);
    *CAPTURED.lock().unwrap() = None;
}

/// 取得錄製結果：None = 還在等；Some(None) = 取消；Some(Some(s)) = 錄到 s
pub fn take_capture() -> Option<Option<String>> {
    CAPTURED.lock().unwrap().take()
}

// ---------------------------------------------------------------- Hook

fn input_index(t: Trigger) -> usize {
    match t {
        Trigger::Key(vk) => (vk as usize) & 0xFF,
        Trigger::MouseMiddle => 256,
        Trigger::MouseX1 => 257,
        Trigger::MouseX2 => 258,
    }
}

fn target_matches(target: &str) -> bool {
    target.is_empty()
        || win::foreground_process_name()
            .map(|n| n.to_lowercase() == target.to_lowercase())
            .unwrap_or(false)
}

/// 回傳 true 表示攔截此事件（不傳給其他程式）
fn handle_input(input: Trigger, down: bool) -> bool {
    let idx = input_index(input);
    let mut pressed = DOWN.lock().unwrap();
    if !down {
        return std::mem::replace(&mut pressed[idx], false);
    }
    if pressed[idx] {
        return true; // 長按的重複事件
    }

    if CAPTURING.load(Ordering::SeqCst) {
        if let Trigger::Key(vk) = input {
            if hotkey::is_modifier(vk) {
                return false; // 讓使用者先按住修飾鍵
            }
        }
        let result = if input == Trigger::Key(hotkey::VK_ESCAPE) {
            None
        } else {
            match hotkey::format_combo(hotkey::current_modifiers(), input) {
                Some(s) => Some(s),
                None => return true, // 不支援的按鍵：吃掉，繼續等
            }
        };
        CAPTURING.store(false, Ordering::SeqCst);
        *CAPTURED.lock().unwrap() = Some(result);
        pressed[idx] = true;
        repaint();
        return true;
    }

    if !ENABLED.load(Ordering::SeqCst) {
        return false;
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
        if kb.dwExtraInfo != win::INJECT_TAG {
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if (down || up) && handle_input(Trigger::Key(kb.vkCode as u16), down) {
                return 1;
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let ms = &*(lparam as *const MSLLHOOKSTRUCT);
        if ms.dwExtraInfo != win::INJECT_TAG {
            let xbtn = || if (ms.mouseData >> 16) & 0xFFFF == 1 { Trigger::MouseX1 } else { Trigger::MouseX2 };
            let event = match wparam as u32 {
                WM_MBUTTONDOWN => Some((Trigger::MouseMiddle, true)),
                WM_MBUTTONUP => Some((Trigger::MouseMiddle, false)),
                WM_XBUTTONDOWN => Some((xbtn(), true)),
                WM_XBUTTONUP => Some((xbtn(), false)),
                _ => None,
            };
            if let Some((t, down)) = event {
                if handle_input(t, down) {
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

// ---------------------------------------------------------------- 動作

fn do_exit() {
    let (cfg, cursor_vk) = {
        let g = RUNTIME.read().unwrap();
        let rt = g.as_ref().unwrap();
        (rt.cfg.clone(), rt.cursor_vk)
    };
    let Some(area) = win::foreground_client() else {
        log("失敗：找不到前景視窗");
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

    let (sx, sy) = area.ratio_to_screen(cfg.exit_button.x, cfg.exit_button.y);
    win::move_mouse(sx, sy);
    win::sleep_ms(cfg.click.move_delay_ms);
    win::left_click(cfg.click.hold_ms);

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
    log(format!("已點擊退出按鈕（螢幕座標 {sx},{sy}，畫面 {}x{}）", area.width, area.height));
}

fn do_calibrate() {
    let (Some(area), Some((x, y))) = (win::foreground_client(), win::cursor_pos()) else {
        log("校正失敗：找不到前景視窗或滑鼠位置");
        beep(false);
        return;
    };
    let (rx, ry) = area.screen_to_ratio(x, y);
    if !(0.0..=1.0).contains(&rx) || !(0.0..=1.0).contains(&ry) {
        log("校正失敗：滑鼠不在前景視窗範圍內");
        beep(false);
        return;
    }

    let mut cfg = current_config();
    cfg.exit_button.x = rx;
    cfg.exit_button.y = ry;

    let mut bound = None;
    if cfg.target_process.trim().is_empty() {
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

    match update_config(cfg) {
        Ok(()) => {
            GENERATION.fetch_add(1, Ordering::SeqCst);
            log(format!("已校正退出按鈕位置：x={rx:.4}, y={ry:.4}（畫面 {}x{}）", area.width, area.height));
            if let Some(name) = bound {
                log(format!("已自動綁定目標程式：{name}"));
            }
            beep(true);
        }
        Err(e) => {
            log(format!("校正失敗：{e}"));
            beep(false);
        }
    }
}

// ---------------------------------------------------------------- 啟動

/// 套用初始設定並啟動背景執行緒（Hook 執行緒 + 動作執行緒）
pub fn start(cfg: Config) -> Result<(), String> {
    apply(cfg)?;

    let (tx, rx) = channel::<Action>();
    TX.set(tx).map_err(|_| "引擎已啟動".to_string())?;
    std::thread::spawn(move || {
        for action in rx {
            match action {
                Action::Exit => do_exit(),
                Action::Calibrate => do_calibrate(),
            }
        }
    });

    // Low-level hook 必須在有訊息迴圈的執行緒上安裝；獨立一條，避免畫面重繪造成輸入延遲
    let (ready_tx, ready_rx) = channel::<Result<(), String>>();
    std::thread::spawn(move || unsafe {
        let hmod = GetModuleHandleW(std::ptr::null());
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
        if kb.is_null() || ms.is_null() {
            let _ = ready_tx.send(Err("無法安裝鍵盤/滑鼠監聽".into()));
            return;
        }
        let _ = ready_tx.send(Ok(()));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    ready_rx.recv().map_err(|_| "監聽執行緒異常結束".to_string())?
}
