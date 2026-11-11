//! 背景引擎：全域鍵盤/滑鼠 Hook、快捷鍵錄製、自動點擊、校正、設定儲存、事件紀錄。
//! GUI 只透過這裡的公開函式讀寫狀態。

use std::collections::VecDeque;
use std::ptr::null_mut;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock, RwLock};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, SYSTEMTIME, WPARAM};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION,
    KBDLLHOOKSTRUCT, MB_ICONASTERISK, MB_ICONHAND, MB_OK, MSG, MSLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_SYSKEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::config::{self, Config, CursorMode};
use crate::hotkey::{self, Hotkey, Trigger};
use crate::win;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    /// 按了快捷鍵卻沒觸發：寫入診斷紀錄（Hook 回呼中不能直接寫檔）
    Miss(String),
    Exit,
    /// 按下校正快捷鍵：進入「等待左鍵點擊」狀態
    Calibrate,
    /// 等待中按下了左鍵（螢幕座標）
    CalibrateAt(i32, i32),
    /// 等待中按了 Esc
    CalibrateCancel,
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
struct KeyState {
    /// 已觸發而尚未放開的按鍵（避免長按連發，並一併攔截放開事件）
    pressed: [bool; 260],
    /// 實體按住中的按鍵（不論有沒有觸發），用來辨識自動連發
    held: [bool; 260],
    /// 最後一次按下事件的時間（毫秒，來自 Hook 事件）
    last_down: [u32; 260],
}
static KEYS: Mutex<KeyState> =
    Mutex::new(KeyState { pressed: [false; 260], held: [false; 260], last_down: [0; 260] });
/// 鍵盤自動連發的間隔最長約 1 秒（初始延遲）；超過這個間隔還被當成按住，表示漏收了放開事件
const MISSED_UP_GAP_MS: u32 = 1200;
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// 診斷用：Hook 收到的鍵盤事件數與最後按下的鍵
static KEY_EVENTS: AtomicU64 = AtomicU64::new(0);
static LAST_VK: AtomicU32 = AtomicU32::new(0);
static REPAINT: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// 快捷鍵錄製：CAPTURING 為 true 時，下一個按鍵會被記錄到 CAPTURED
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// 校正：等待使用者用左鍵點擊退出按鈕
static CALIBRATING: AtomicBool = AtomicBool::new(false);
/// 每次進入等待狀態遞增，用來判斷逾時計時器是否屬於目前這一次
static CALIBRATE_ID: AtomicU64 = AtomicU64::new(0);
/// 校正用的那次左鍵已攔截，對應的放開事件也要攔截
static SWALLOW_LBUTTON_UP: AtomicBool = AtomicBool::new(false);
const CALIBRATE_TIMEOUT_SECS: u64 = 20;
/// Some(Some(s)) = 錄到 s；Some(None) = 使用者按 Esc 取消
static CAPTURED: Mutex<Option<Option<String>>> = Mutex::new(None);

// ---------------------------------------------------------------- 紀錄 / 通知

fn now() -> String {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
}

/// 寫入紀錄（畫面 + exe 旁的 QuickExit.log）。不可在 Hook 回呼中呼叫。
pub fn log(msg: impl Into<String>) {
    let line = format!("[{}] {}", now(), msg.into());
    let path = config::config_path().with_file_name("QuickExit.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
    let mut l = LOG.lock().unwrap();
    l.push_back(line);
    while l.len() > 200 {
        l.pop_front();
    }
    drop(l);
    repaint();
}

pub fn logs() -> Vec<String> {
    LOG.lock().unwrap().iter().cloned().collect()
}

/// (鍵盤事件總數, 最後按下的虛擬鍵碼)
pub fn key_stats() -> (u64, u16) {
    (KEY_EVENTS.load(Ordering::Relaxed), LAST_VK.load(Ordering::Relaxed) as u16)
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

/// 是否正在等待校正點擊
pub fn calibrating() -> bool {
    CALIBRATING.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------- 快捷鍵錄製

pub fn start_capture() {
    *CAPTURED.lock().unwrap() = None;
    CAPTURING.store(true, Ordering::SeqCst);
    std::thread::spawn(poll_capture);
}

/// 備援的錄製方式：直接輪詢鍵盤狀態（不依賴 Hook）。
/// 錄製時本程式視窗在前景，按鍵只會送到自己，不吃掉也沒關係。
fn poll_capture() {
    // 候選按鍵：可命名的非修飾鍵（不含滑鼠左右鍵），加上滑鼠中鍵 / 側鍵
    let mut candidates: Vec<(u16, Trigger)> = (0x07u16..=0xFE)
        .filter(|&vk| !hotkey::is_modifier(vk))
        .filter_map(|vk| hotkey::key_name(Trigger::Key(vk)).map(|_| (vk, Trigger::Key(vk))))
        .collect();
    candidates.push((0x04, Trigger::MouseMiddle));
    candidates.push((0x05, Trigger::MouseX1));
    candidates.push((0x06, Trigger::MouseX2));

    // 開始時已經按著的鍵不算，必須「放開後再按下」
    let mut was_down: Vec<bool> = candidates.iter().map(|&(vk, _)| win::key_is_down(vk)).collect();
    while CAPTURING.load(Ordering::SeqCst) {
        for (i, &(vk, t)) in candidates.iter().enumerate() {
            let down = win::key_is_down(vk);
            if down && !was_down[i] {
                let result = if vk == hotkey::VK_ESCAPE {
                    None
                } else {
                    hotkey::format_combo(hotkey::current_modifiers(), t)
                };
                // 用 swap 確保只有一條路徑（Hook 或輪詢）寫入結果
                if CAPTURING.swap(false, Ordering::SeqCst) {
                    *CAPTURED.lock().unwrap() = Some(result);
                    repaint();
                }
                return;
            }
            was_down[i] = down;
        }
        win::sleep_ms(10);
    }
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

/// 前景是否為目標程式；不是的話回傳原因
fn target_check(target: &str) -> Result<(), String> {
    if target.is_empty() {
        return Ok(());
    }
    let (name, title) =
        win::foreground_process().map_err(|e| format!("無法確認前景是不是 {target}：{e}"))?;
    if name.to_lowercase() == target.to_lowercase() {
        Ok(())
    } else {
        Err(format!("前景視窗是 {name}（「{title}」），不是目標程式 {target}"))
    }
}

fn send(action: Action) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(action);
    }
}

fn input_name(t: Trigger) -> String {
    hotkey::key_name(t).unwrap_or_else(|| match t {
        Trigger::Key(vk) => format!("按鍵 {vk:#04X}"),
        other => format!("{other:?}"),
    })
}

/// 回傳 true 表示攔截此事件（不傳給其他程式）。time 是事件時間（毫秒）。
fn handle_input(input: Trigger, down: bool, time: u32) -> bool {
    let idx = input_index(input);
    let mut keys = KEYS.lock().unwrap();
    if !down {
        keys.held[idx] = false;
        return std::mem::replace(&mut keys.pressed[idx], false);
    }
    let gap = time.wrapping_sub(keys.last_down[idx]);
    keys.last_down[idx] = time;
    // 滑鼠按鍵不會自動連發；鍵盤連發的間隔不會超過 MISSED_UP_GAP_MS
    let is_key = matches!(input, Trigger::Key(_));
    let repeat = is_key && std::mem::replace(&mut keys.held[idx], true) && gap <= MISSED_UP_GAP_MS;
    keys.held[idx] = true;
    // 連發的重複事件不重複記錄
    let miss = |msg: String| {
        if !repeat {
            send(Action::Miss(format!("沒觸發：{msg}")));
        }
    };

    if keys.pressed[idx] {
        miss(format!(
            "{} 被當成長按而略過（沒收到上一次的放開事件，距上次按下 {:.1} 秒）",
            input_name(input),
            gap as f64 / 1000.0
        ));
        return true; // 長按的重複事件
    }

    // debug 自我測試：停用 Hook 錄製，只測輪詢備援
    if cfg!(debug_assertions) && CAPTURING.load(Ordering::SeqCst) && std::env::var_os("QUICKEXIT_SELFTEST_POLL").is_some() {
        return false;
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
        if CAPTURING.swap(false, Ordering::SeqCst) {
            *CAPTURED.lock().unwrap() = Some(result);
        }
        keys.pressed[idx] = true;
        drop(keys);
        repaint();
        return true;
    }

    if input == Trigger::Key(hotkey::VK_ESCAPE) && CALIBRATING.swap(false, Ordering::SeqCst) {
        keys.pressed[idx] = true;
        send(Action::CalibrateCancel);
        return true;
    }

    if !ENABLED.load(Ordering::SeqCst) {
        return false;
    }
    let b = BINDINGS.read().unwrap();
    let mods = hotkey::current_modifiers();
    let hit = b.list.iter().find(|(hk, _)| hk.trigger_matches(input) && hk.modifiers_match(mods));
    let Some((hk, action)) = hit else {
        // 觸發鍵對了、修飾鍵不對：記下實際按的組合
        let wanted: Vec<&str> =
            b.list.iter().filter(|(hk, _)| hk.trigger_matches(input)).map(|(hk, _)| hk.text.as_str()).collect();
        if !wanted.is_empty() {
            let got = hotkey::format_combo(mods, input).unwrap_or_else(|| input_name(input));
            miss(format!("按下的是 {got}，但設定是 {}（修飾鍵不符）", wanted.join(" / ")));
        }
        return false;
    };
    if let Err(reason) = target_check(&b.target) {
        miss(format!("{}：{reason}", hk.text));
        return false;
    }
    keys.pressed[idx] = true;
    send(action.clone());
    true
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        KEY_EVENTS.fetch_add(1, Ordering::Relaxed);
        if wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN {
            LAST_VK.store(kb.vkCode, Ordering::Relaxed);
        }
        if kb.dwExtraInfo != win::INJECT_TAG {
            let msg = wparam as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if (down || up) && handle_input(Trigger::Key(kb.vkCode as u16), down, kb.time) {
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
            match wparam as u32 {
                // 校正等待中的左鍵：記錄位置並攔截（不讓遊戲真的點到退出）
                WM_LBUTTONDOWN if CALIBRATING.swap(false, Ordering::SeqCst) => {
                    SWALLOW_LBUTTON_UP.store(true, Ordering::SeqCst);
                    if let Some(tx) = TX.get() {
                        let _ = tx.send(Action::CalibrateAt(ms.pt.x, ms.pt.y));
                    }
                    return 1;
                }
                WM_LBUTTONUP if SWALLOW_LBUTTON_UP.swap(false, Ordering::SeqCst) => return 1,
                _ => {}
            }
            let event = match wparam as u32 {
                WM_MBUTTONDOWN => Some((Trigger::MouseMiddle, true)),
                WM_MBUTTONUP => Some((Trigger::MouseMiddle, false)),
                WM_XBUTTONDOWN => Some((xbtn(), true)),
                WM_XBUTTONUP => Some((xbtn(), false)),
                _ => None,
            };
            if let Some((t, down)) = event {
                if handle_input(t, down, ms.time) {
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

fn calibrate_arm() {
    let id = CALIBRATE_ID.fetch_add(1, Ordering::SeqCst) + 1;
    CALIBRATING.store(true, Ordering::SeqCst);
    log(format!(
        "校正中：請在遊戲中叫出滑鼠，用左鍵點一下退出按鈕（這次點擊不會傳給遊戲；{CALIBRATE_TIMEOUT_SECS} 秒內有效，Esc 取消）"
    ));
    unsafe { MessageBeep(MB_ICONASTERISK) };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(CALIBRATE_TIMEOUT_SECS));
        if CALIBRATE_ID.load(Ordering::SeqCst) == id && CALIBRATING.swap(false, Ordering::SeqCst) {
            log("校正已逾時取消");
            beep(false);
        }
    });
}

fn do_calibrate(x: i32, y: i32) {
    if win::foreground_is_self() {
        log("已取消校正（點擊了 QuickExit 視窗）");
        return;
    }
    let Some(area) = win::foreground_client() else {
        log("校正失敗：找不到前景視窗");
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
                Action::Miss(msg) => log(msg),
                Action::Exit => do_exit(),
                Action::Calibrate => calibrate_arm(),
                Action::CalibrateAt(x, y) => do_calibrate(x, y),
                Action::CalibrateCancel => log("已取消校正"),
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
    ready_rx.recv().map_err(|_| "監聽執行緒異常結束".to_string())??;
    log(format!("鍵盤/滑鼠監聽已啟動（{}）", if win::is_admin() { "系統管理員" } else { "一般使用者" }));
    Ok(())
}
