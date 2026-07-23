mod app;
mod edit;
mod index;
mod indexer;
mod parse;
mod save;
mod sort;
mod source;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "textViewer",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);
            Ok(Box::new(app::App::default()))
        }),
    )
}

/// Windows에 기본 설치된 한글 폰트(맑은 고딕)를 egui에 등록한다.
/// egui 기본 폰트에는 한글 글리프가 없어 한글이 두부(□)로 표시되므로,
/// 시스템 폰트를 런타임에 읽어 proportional/monospace 양쪽 맨 앞에 넣는다.
/// 폰트 파일을 못 찾으면 조용히 넘어가 기본 폰트를 그대로 쓴다(크래시 없음).
fn install_korean_font(ctx: &egui::Context) {
    // 후보 경로: 맑은 고딕(우선), 굴림 순으로 시도.
    let candidates = [
        r"C:\Windows\Fonts\malgun.ttf",
        r"C:\Windows\Fonts\malgunsl.ttf",
        r"C:\Windows\Fonts\gulim.ttc",
    ];
    let font_bytes = candidates
        .iter()
        .find_map(|p| std::fs::read(p).ok());

    let Some(bytes) = font_bytes else {
        return; // 시스템 폰트를 못 찾으면 기본 폰트 유지
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("korean".to_owned(), egui::FontData::from_owned(bytes));

    // proportional과 monospace 모두 맨 앞에 넣어 한글을 최우선 렌더링.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "korean".to_owned());
    }

    ctx.set_fonts(fonts);

    // 본문/버튼 텍스트 크기를 살짝 키워 한글(맑은 고딕)이 작은 크기에서
    // 뭉개져 보이는 것을 완화한다.
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(15.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(15.0, FontFamily::Monospace),
        );
    });
}
