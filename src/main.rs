mod parse;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "textViewer",
        options,
        Box::new(|_cc| Ok(Box::new(PlaceholderApp))),
    )
}

struct PlaceholderApp;

impl eframe::App for PlaceholderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("textViewer");
        });
    }
}
