#![windows_subsystem = "windows"]

mod app;
mod glass;
mod theme;

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Velter")
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "Velter",
        opts,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
