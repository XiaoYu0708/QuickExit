//! 設定視窗（egui）

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::config::{self, Config, CursorMode};
use crate::{engine, win};

const CURSOR_KEYS: &[&str] = &["LCtrl", "RCtrl", "LAlt", "RAlt", "LShift", "RShift"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Exit,
    Calibrate,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Exit => "自動退出",
            Field::Calibrate => "校正按鈕位置",
        }
    }
}

pub struct App {
    /// 畫面上正在編輯的設定
    draft: Config,
    /// 最後一次成功套用的設定
    applied: Config,
    /// 套用失敗的設定（避免每一幀重試）
    failed: Option<Config>,
    generation: u64,
    capturing: Option<Field>,
    error: Option<String>,
    saved_at: Option<Instant>,
    is_admin: bool,
    config_path: String,
    /// debug 版自我測試（環境變數 QUICKEXIT_SELFTEST=1）
    selftest: Option<(Instant, bool)>,
}

impl App {
    pub fn new() -> Self {
        let cfg = engine::current_config();
        Self {
            draft: cfg.clone(),
            applied: cfg,
            failed: None,
            generation: engine::generation(),
            capturing: None,
            error: None,
            saved_at: None,
            is_admin: win::is_admin(),
            config_path: config::config_path().display().to_string(),
            selftest: (cfg!(debug_assertions) && std::env::var_os("QUICKEXIT_SELFTEST").is_some())
                .then(|| (Instant::now(), false)),
        }
    }

    fn sync_from_engine(&mut self) {
        let g = engine::generation();
        if g != self.generation {
            self.generation = g;
            self.draft = engine::current_config();
            self.applied = self.draft.clone();
            self.failed = None;
            self.error = None;
        }

        if let Some(field) = self.capturing {
            if let Some(result) = engine::take_capture() {
                self.capturing = None;
                match result {
                    Some(s) => {
                        engine::log(format!("{}快捷鍵已設定為 {s}", field.label()));
                        match field {
                            Field::Exit => self.draft.hotkeys.exit = s,
                            Field::Calibrate => self.draft.hotkeys.calibrate_exit = s,
                        }
                    }
                    None => engine::log("已取消錄製快捷鍵"),
                }
            }
        }
    }

    fn apply_changes(&mut self, ctx: &egui::Context) {
        // 拖曳數值時先不存，放開滑鼠才存
        if self.draft == self.applied || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if self.failed.as_ref() == Some(&self.draft) {
            return;
        }
        match engine::update_config(self.draft.clone()) {
            Ok(()) => {
                self.applied = self.draft.clone();
                self.failed = None;
                self.error = None;
                self.saved_at = Some(Instant::now());
            }
            Err(e) => {
                self.failed = Some(self.draft.clone());
                self.error = Some(e);
            }
        }
    }

    fn hotkey_button(&mut self, ui: &mut egui::Ui, field: Field) {
        let active = self.capturing == Some(field);
        let value = match field {
            Field::Exit => &self.draft.hotkeys.exit,
            Field::Calibrate => &self.draft.hotkeys.calibrate_exit,
        };
        let text = if active {
            RichText::new("請按下按鍵…（Esc 取消）").color(Color32::from_rgb(230, 160, 40))
        } else {
            RichText::new(value.as_str()).strong()
        };
        if ui.add(egui::Button::new(text).min_size(egui::vec2(190.0, 26.0))).clicked() {
            if active {
                engine::cancel_capture();
                self.capturing = None;
                engine::log("已取消錄製快捷鍵");
            } else {
                engine::start_capture();
                self.capturing = Some(field);
                engine::log(format!("請按下要設定為「{}」的按鍵…", field.label()));
            }
        }
    }

    fn status_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            let mut on = engine::enabled();
            if ui.checkbox(&mut on, RichText::new("啟用快捷鍵").strong()).changed() {
                engine::set_enabled(on);
            }
            ui.separator();
            if self.is_admin {
                ui.colored_label(Color32::from_rgb(80, 180, 90), "● 系統管理員");
            } else {
                ui.colored_label(Color32::from_rgb(230, 160, 40), "● 一般使用者");
                if ui.button("以管理員身分重新啟動").clicked() {
                    win::release_single_instance();
                    if win::relaunch_as_admin() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        win::acquire_single_instance();
                        self.error = Some("未取得系統管理員權限".into());
                    }
                }
            }
        });
        if !self.is_admin {
            ui.label(RichText::new("遊戲若以系統管理員身分執行，本程式也需要管理員權限才能點擊。").weak().small());
        }
    }

    fn hotkey_section(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("hotkeys").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("自動退出");
            self.hotkey_button(ui, Field::Exit);
            ui.end_row();
            ui.label("校正按鈕位置");
            self.hotkey_button(ui, Field::Calibrate);
            ui.end_row();
        });
        ui.label(
            RichText::new("點按鈕後按下想用的按鍵，可搭配 Ctrl / Alt / Shift，也支援滑鼠中鍵與側鍵。")
                .weak()
                .small(),
        );
    }

    fn cursor_section(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.draft.cursor;
        let mode_label = |m: CursorMode| match m {
            CursorMode::Hold => "按住喚出鍵（推薦）",
            CursorMode::Tap => "按一下喚出鍵",
            CursorMode::None => "不按鍵",
        };
        egui::Grid::new("cursor").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("方式");
            egui::ComboBox::from_id_salt("cursor_mode")
                .width(190.0)
                .selected_text(mode_label(d.mode))
                .show_ui(ui, |ui| {
                    for m in [CursorMode::Hold, CursorMode::Tap, CursorMode::None] {
                        ui.selectable_value(&mut d.mode, m, mode_label(m));
                    }
                });
            ui.end_row();

            if d.mode != CursorMode::None {
                ui.label("喚出鍵");
                egui::ComboBox::from_id_salt("cursor_key")
                    .width(190.0)
                    .selected_text(d.key.as_str())
                    .show_ui(ui, |ui| {
                        for k in CURSOR_KEYS {
                            ui.selectable_value(&mut d.key, k.to_string(), *k);
                        }
                    });
                ui.end_row();

                ui.label("等待滑鼠出現");
                ui.add(egui::DragValue::new(&mut d.reveal_delay_ms).range(0..=3000).suffix(" ms"));
                ui.end_row();
            }

            if d.mode == CursorMode::Tap {
                ui.label("");
                ui.checkbox(&mut d.hide_after, "點完後再按一次收起滑鼠");
                ui.end_row();
            }
        });
        let hint = match d.mode {
            CursorMode::Hold => "點擊期間一直按住喚出鍵。遊戲設定「按住 Ctrl」或「按一下 Ctrl」都適用。",
            CursorMode::Tap => "先按一下喚出鍵再點擊。適用遊戲設定為「按一下 Ctrl」切換滑鼠。",
            CursorMode::None => "不按任何鍵，適用滑鼠本來就看得到的情況。",
        };
        ui.label(RichText::new(hint).weak().small());
    }

    fn click_section(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.draft;
        egui::Grid::new("click").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("退出按鈕位置");
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(&mut d.exit_button.x).range(0.0..=1.0).speed(0.0005).fixed_decimals(4));
                ui.label("Y");
                ui.add(egui::DragValue::new(&mut d.exit_button.y).range(0.0..=1.0).speed(0.0005).fixed_decimals(4));
            });
            ui.end_row();

            ui.label("移到按鈕後等待");
            ui.add(egui::DragValue::new(&mut d.click.move_delay_ms).range(0..=2000).suffix(" ms"));
            ui.end_row();

            ui.label("左鍵按住");
            ui.add(egui::DragValue::new(&mut d.click.hold_ms).range(0..=1000).suffix(" ms"));
            ui.end_row();

            ui.label("");
            ui.checkbox(&mut d.click.restore_cursor, "點完把滑鼠移回原位");
            ui.end_row();
        });
        ui.label(
            RichText::new(format!(
                "校正：在遊戲中叫出滑鼠，移到右上角退出按鈕上，按「{}」，聽到提示音就完成了。",
                d.hotkeys.calibrate_exit
            ))
            .weak()
            .small(),
        );
    }

    fn target_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.draft.target_process)
                    .hint_text("留空 = 任何視窗")
                    .desired_width(220.0),
            );
            if ui.button("清除").clicked() {
                self.draft.target_process.clear();
            }
        });
        ui.label(RichText::new("只有這個程式在前景時快捷鍵才生效；第一次校正時會自動填入。").weak().small());
        ui.add_space(4.0);
        ui.checkbox(&mut self.draft.run_as_admin, "啟動時自動要求系統管理員權限");
    }

    fn log_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("紀錄");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("清除").clicked() {
                    engine::clear_logs();
                }
            });
        });
        let lines = engine::logs();
        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(130.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if lines.is_empty() {
                    ui.label(RichText::new("（尚無紀錄）").weak().small());
                }
                for l in lines {
                    ui.label(RichText::new(l).small().monospace());
                }
            });
    }
}

impl App {
    /// 自我測試：進入錄製 → 模擬外部按下 F24 → 檢查是否錄到
    fn run_selftest(&mut self, ctx: &egui::Context) {
        let Some((t0, sent)) = self.selftest.as_mut() else { return };
        let ms = t0.elapsed().as_millis();
        if ms < 500 {
            if self.capturing.is_none() {
                engine::start_capture();
                self.capturing = Some(Field::Exit);
            }
        } else if !*sent && ms >= 1000 {
            *sent = true;
            std::thread::spawn(|| {
                win::send_key_tagged(0x87, false, 0);
                win::sleep_ms(30);
                win::send_key_tagged(0x87, true, 0);
            });
        } else if ms >= 2500 {
            println!("[selftest] RESULT exit={}", self.draft.hotkeys.exit);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        println!(
            "[selftest] t={ms} focused={} capturing={} exit={}",
            ctx.input(|i| i.focused),
            self.capturing.is_some(),
            self.draft.hotkeys.exit
        );
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !title.is_empty() {
            ui.strong(title);
            ui.add_space(6.0);
        }
        add(ui);
    });
    ui.add_space(6.0);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_from_engine();
        self.run_selftest(ctx);

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("設定檔：{}", self.config_path)).weak().small());
                if let Some(t) = self.saved_at {
                    if t.elapsed() < Duration::from_secs(2) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("已自動儲存").small().color(Color32::from_rgb(80, 180, 90)));
                        });
                        ctx.request_repaint_after(Duration::from_millis(500));
                    }
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.status_section(ui, ctx);
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::from_rgb(220, 70, 70), format!("⚠ {e}"));
                }
                ui.add_space(6.0);
                section(ui, "快捷鍵", |ui| self.hotkey_section(ui));
                section(ui, "喚出滑鼠", |ui| self.cursor_section(ui));
                section(ui, "點擊", |ui| self.click_section(ui));
                section(ui, "目標程式", |ui| self.target_section(ui));
                section(ui, "", |ui| self.log_section(ui));
            });
        });

        self.apply_changes(ctx);
        if self.capturing.is_some() {
            // 切換到其他程式（例如回到遊戲）就取消錄製，避免把遊戲中的按鍵吃掉。
            // 用 Win32 前景視窗判斷，不用 egui 的 focused（實測不可靠，會導致一按就被取消）。
            if !win::foreground_is_self() && self.selftest.is_none() {
                engine::cancel_capture();
                self.capturing = None;
                engine::log("已取消錄製快捷鍵（切換到其他視窗）");
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

/// 視窗圖示：圓角方塊上的「離開」符號（程式繪製，不需要圖檔）
pub fn icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let r = 14.0;
            let dx = ((fx - 32.0).abs() - (30.0 - r)).max(0.0);
            let dy = ((fy - 32.0).abs() - (30.0 - r)).max(0.0);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let inr = |x0: f32, x1: f32, y0: f32, y1: f32| fx >= x0 && fx <= x1 && fy >= y0 && fy <= y1;
            let door = inr(14.0, 19.0, 14.0, 50.0) || inr(14.0, 34.0, 14.0, 19.0) || inr(14.0, 34.0, 45.0, 50.0);
            let shaft = inr(24.0, 44.0, 29.5, 34.5);
            let head = fx >= 42.0 && fx <= 54.0 && (fy - 32.0).abs() <= (54.0 - fx) * 10.0 / 12.0;
            let c: [u8; 4] = if door || shaft || head { [255, 255, 255, 255] } else { [196, 64, 56, 255] };
            rgba[(y * N + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}

/// 載入系統中文字型（egui 內建字型沒有中文）
pub fn setup_fonts(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let mut fonts = egui::FontDefinitions::default();
    for file in ["msjh.ttc", "msyh.ttc", "mingliu.ttc", "simsun.ttc"] {
        if let Ok(bytes) = std::fs::read(format!("{windir}\\Fonts\\{file}")) {
            fonts.font_data.insert("cjk".into(), egui::FontData::from_owned(bytes));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".into());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
    ctx.style_mut(|s| {
        for font in s.text_styles.values_mut() {
            font.size += 1.0;
        }
    });
}
