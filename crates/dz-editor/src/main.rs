#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! DZ World Editor: terrain, objects and roads for DayZ maps.
mod app;
mod asset_browser;
mod camera;
mod i18n;
mod library_ui;
mod maskmods;
mod models;
mod panels;
mod render;
mod satellite;
mod settings;
mod theme;
mod updater;
mod viewport;

use eframe::egui;

struct Host {
    app: app::App,
    last: std::time::Instant,
}

impl eframe::App for Host {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = std::time::Instant::now();
        self.app.frame_ms = self.app.frame_ms * 0.9 + (now - self.last).as_secs_f32() * 100.0;
        if let Some(v) = &mut self.app.perf {
            v.push((now - self.last).as_secs_f32() * 1000.0);
        }
        self.last = now;
        self.app.tick();
        panels::ui(&mut self.app, ctx);
        viewport::run_script(&mut self.app, ctx);
        self.app.handle_screenshots(ctx);
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.app.synthetic);
        if self.app.autoscript.is_some() {
            raw.modifiers = self.app.synthetic_modifiers;
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.settings.save();
        self.app.updater.on_exit();
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(format!("DZ World Editor {}", updater::version())).with_inner_size([1600.0, 950.0]).with_min_inner_size([1000.0, 600.0]).with_maximized(std::env::var_os("DZWE_AUTO").is_none()),
        depth_buffer: 24,
        multisampling: 0,
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native("DZ World Editor", options, Box::new(|cc| {
        let mut app = app::App::new(cc.egui_ctx.clone());
        if let Ok(s) = std::env::var("DZWE_AUTO") {
            app.autoscript = Some(viewport::AutoScript { steps: s.split(';').map(str::to_string).collect(), wait: 3 });
        }
        Ok(Box::new(Host { app, last: std::time::Instant::now() }))
    }))
}
