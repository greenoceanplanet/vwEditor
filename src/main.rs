mod app;
mod index;
mod indexer;
mod parse;
mod source;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "textViewer",
        options,
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
