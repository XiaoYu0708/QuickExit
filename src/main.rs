//! QuickExit：按自訂快捷鍵，自動喚出滑鼠並點擊遊戲右上角的「退出」按鈕。

// Release 版不顯示主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod hotkey;
mod win;

use eframe::egui;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TITLE: &str = "QuickExit";

fn main() {
    // 讓所有座標都是實體像素（高 DPI 縮放下點擊位置才會準）
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let path = config::config_path();
    let cfg = match config::load_or_create(&path) {
        Ok((cfg, _)) => cfg,
        Err(e) => {
            win::message_box(&e, TITLE, true);
            return;
        }
    };

    if cfg.run_as_admin && !win::is_admin() {
        if win::relaunch_as_admin() {
            return;
        }
        engine::log("未取得系統管理員權限；若遊戲以管理員身分執行，快捷鍵可能無效。");
    }

    if !win::acquire_single_instance() {
        win::message_box("QuickExit 已經在執行中了。", TITLE, false);
        return;
    }

    if let Err(e) = engine::start(cfg.clone()) {
        // 設定檔有錯：保留位置與目標程式，其餘改回預設值
        let fallback = config::Config {
            target_process: cfg.target_process,
            exit_button: cfg.exit_button,
            run_as_admin: cfg.run_as_admin,
            ..Default::default()
        };
        win::message_box(&format!("設定檔有錯誤，已改用預設快捷鍵：\n{e}"), TITLE, true);
        if let Err(e) = engine::start(fallback) {
            win::message_box(&format!("無法啟動：{e}"), TITLE, true);
            return;
        }
    }
    engine::log(format!("QuickExit v{VERSION} 已啟動"));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{TITLE} v{VERSION} - 副本退出快捷鍵"))
            .with_inner_size([470.0, 720.0])
            .with_min_inner_size([400.0, 420.0])
            .with_icon(app::icon()),
        ..Default::default()
    };
    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(|cc| {
            app::setup_fonts(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            engine::set_repaint(move || ctx.request_repaint());
            Ok(Box::new(app::App::new()))
        }),
    );
    if let Err(e) = result {
        win::message_box(&format!("無法開啟視窗：{e}"), TITLE, true);
    }
}
