mod app;
mod edit;
mod index;
mod indexer;
mod parse;
mod save;
mod sort;
mod source;
mod theme;

fn main() -> eframe::Result<()> {
    // 첫 인자가 있으면 그 파일을 열고 시작한다(셸에서 실행하거나 exe에
    // 파일을 끌어다 놓는 경우). 없으면 빈 상태로 시작.
    let initial = std::env::args().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "textViewer",
        options,
        Box::new(move |cc| {
            // 폰트/텍스트 스타일/Visuals를 한 번에 설치한다(theme.rs).
            theme::install(&cc.egui_ctx);
            let mut app = app::App::default();
            if let Some(p) = initial {
                app.open_path(&p, &cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}
