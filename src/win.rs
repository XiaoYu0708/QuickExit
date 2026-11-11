//! Win32 輔助函式：視窗資訊、模擬鍵盤滑鼠、權限

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::null_mut;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
    MOUSEINPUT,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowTextW, GetWindowThreadProcessId,
    SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOWNORMAL,
};

/// 本程式送出的模擬輸入都帶這個標記，Hook 據此只忽略自己送的事件
/// （其他軟體——巨集鍵盤驅動、AutoHotkey、遠端桌面——送來的按鍵仍然要能觸發快捷鍵）
pub const INJECT_TAG: usize = 0x5145_5854; // "QEXT"

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub fn sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// 視窗工作區（不含標題列與邊框）在螢幕上的位置與大小
#[derive(Clone, Copy, Debug)]
pub struct ClientArea {
    pub hwnd: HWND,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

pub fn foreground_client() -> Option<ClientArea> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut rc: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut rc) == 0 {
            return None;
        }
        let mut pt = POINT { x: 0, y: 0 };
        if ClientToScreen(hwnd, &mut pt) == 0 {
            return None;
        }
        let (width, height) = (rc.right - rc.left, rc.bottom - rc.top);
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(ClientArea { hwnd, left: pt.x, top: pt.y, width, height })
    }
}

impl ClientArea {
    pub fn ratio_to_screen(&self, x: f64, y: f64) -> (i32, i32) {
        (
            self.left + (x * self.width as f64).round() as i32,
            self.top + (y * self.height as f64).round() as i32,
        )
    }

    pub fn screen_to_ratio(&self, x: i32, y: i32) -> (f64, f64) {
        (
            (x - self.left) as f64 / self.width as f64,
            (y - self.top) as f64 / self.height as f64,
        )
    }
}

/// 取得視窗所屬程式的執行檔名稱（例如 "game.exe"）
pub fn process_name(hwnd: HWND) -> Option<String> {
    process_name_detailed(hwnd).ok()
}

/// 同 process_name，失敗時回傳原因（給診斷紀錄用）
fn process_name_detailed(hwnd: HWND) -> Result<String, String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return Err(format!("查不到視窗的 PID（錯誤碼 {}）", GetLastError()));
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return Err(format!("無法開啟程式 PID {pid}（錯誤碼 {}）", GetLastError()));
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        let err = GetLastError();
        CloseHandle(h);
        if ok == 0 {
            return Err(format!("無法查詢程式 PID {pid} 的名稱（錯誤碼 {err}）"));
        }
        let full = OsString::from_wide(&buf[..len as usize]);
        std::path::Path::new(&full)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("程式 PID {pid} 的路徑沒有檔名"))
    }
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// 前景視窗的 (程式名稱, 視窗標題)；失敗時回傳原因
pub fn foreground_process() -> Result<(String, String), String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Err("目前沒有前景視窗（可能正在切換視窗）".into());
    }
    let title = window_title(hwnd);
    process_name_detailed(hwnd)
        .map(|name| (name, title.clone()))
        .map_err(|e| if title.is_empty() { e } else { format!("{e}，視窗「{title}」") })
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut pt = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut pt) } != 0).then_some((pt.x, pt.y))
}

pub fn key_is_down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// 以掃描碼送出按鍵（DirectInput / Raw Input 遊戲也收得到）
pub fn send_key(vk: u16, up: bool) {
    send_key_tagged(vk, up, INJECT_TAG);
}

/// tag = 0 時送出「像外部軟體送的」按鍵（自我測試用）
pub fn send_key_tagged(vk: u16, up: bool, tag: usize) {
    // MAPVK_VK_TO_VSC_EX = 4：延伸鍵會回傳 0xE0xx
    let sc = unsafe { MapVirtualKeyW(vk as u32, 4) };
    let mut flags = KEYEVENTF_SCANCODE;
    if sc & 0xFF00 == 0xE000 || sc & 0xFF00 == 0xE100 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: (sc & 0xFF) as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: tag,
            },
        },
    };
    send(&[input]);
}

pub fn tap_key(vk: u16) {
    send_key(vk, false);
    sleep_ms(40);
    send_key(vk, true);
}

fn mouse_input(dx: i32, dy: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECT_TAG,
            },
        },
    }
}

/// 移動滑鼠到螢幕座標（同時用 SetCursorPos 與 SendInput，讓遊戲確實收到移動事件）
pub fn move_mouse(x: i32, y: i32) {
    unsafe {
        SetCursorPos(x, y);
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2);
        let nx = ((x - vx) as f64 * 65535.0 / (vw - 1) as f64).round() as i32;
        let ny = ((y - vy) as f64 * 65535.0 / (vh - 1) as f64).round() as i32;
        send(&[mouse_input(
            nx,
            ny,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        )]);
    }
}

pub fn left_click(hold_ms: u64) {
    send(&[mouse_input(0, 0, MOUSEEVENTF_LEFTDOWN)]);
    sleep_ms(hold_ms);
    send(&[mouse_input(0, 0, MOUSEEVENTF_LEFTUP)]);
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

/// 以系統管理員身分重新啟動自己。成功送出要求回傳 true
pub fn relaunch_as_admin() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb = wide("runas");
    let r = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}

// ---------------------------------------------------------------- 單一執行個體 / 訊息框

static INSTANCE_MUTEX: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
const INSTANCE_NAME: &str = r"Local\QuickExit_SingleInstance";

/// 取得單一執行個體鎖；已經有其他 QuickExit 在執行時回傳 false
pub fn acquire_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = wide(INSTANCE_NAME);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if h.is_null() {
            return true; // 建立失敗就不限制
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return false;
        }
        *INSTANCE_MUTEX.lock().unwrap() = h as usize;
    }
    true
}

/// 釋放單一執行個體鎖（以管理員身分重新啟動前呼叫，讓新的程式能啟動）
pub fn release_single_instance() {
    let mut h = INSTANCE_MUTEX.lock().unwrap();
    if *h != 0 {
        unsafe { CloseHandle(*h as _) };
        *h = 0;
    }
}

pub fn message_box(text: &str, title: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};
    let (t, c) = (wide(text), wide(title));
    unsafe {
        MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION });
    }
}

/// 前景視窗是否屬於本程式
pub fn foreground_is_self() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid == GetCurrentProcessId()
    }
}
