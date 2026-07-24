//! 앱 전체의 룩앤필(폰트·텍스트 스타일·Visuals·표 색상)을 한곳에 모은다.
//!
//! **왜 한 파일인가.** 폰트 등록은 `main.rs`에, 색상은 `app.rs`에 흩어져 있으면
//! "표 셀이 왜 가변폭으로 그려지는가" 같은 질문의 답이 두 파일에 나뉘어 있게
//! 된다. 화면 인상을 결정하는 값은 전부 여기 있고, `app.rs`는 그리기만 한다.
//!
//! 설계 원칙:
//! - **데이터 영역은 고정폭**(Cascadia Mono). 숫자 자릿수가 세로로 맞아야
//!   데이터 도구처럼 보인다. 한글은 고정폭 영문 폰트에 글리프가 없으므로
//!   **뒤에 폴백으로** 맑은 고딕을 붙인다(앞에 붙이면 영문·숫자까지 가변폭이
//!   되어 정렬이 깨진다 — 이것이 이전 구현의 문제였다).
//! - **UI 크롬은 Segoe UI**(Windows 표준). 메뉴·버튼·상태바.
//! - 폰트 파일이 없으면 그 단계만 건너뛰고 egui 내장 폰트로 떨어진다(크래시 없음).

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Stroke, TextStyle};

/// 표/텍스트 모드 한 행의 높이. (`app.rs`의 `ROW_HEIGHT`가 이 값을 쓴다 —
/// 격자선 간격과 직결되므로 폰트 크기와 같은 곳에서 관리한다.)
///
/// **왜 22인가.** 13px 고정폭의 실제 행 높이는 약 17~18px이고, 인라인 셀 편집기
/// (`TextEdit::singleline`)는 거기에 세로 마진 2+2px을 더해 자리를 잡는다
/// (`egui-0.28.1/src/widgets/text_edit/builder.rs:123, 510`). 그보다 낮게 잡으면
/// 더블클릭으로 셀 편집을 시작한 순간 편집기가 행 높이를 넘겨 잘려 보인다.
/// 글꼴이 15px→13px로 작아졌으므로 같은 22에서도 이전보다 촘촘해 보인다.
pub const ROW_HEIGHT: f32 = 22.0;

/// 데이터 영역 고정폭 폰트 크기. `TextStyle::Body`/`Monospace`와
/// `app.rs`의 텍스트 모드 galley 레이아웃이 같은 값을 써야 캐럿이 글자와
/// 어긋나지 않는다.
pub const MONO_SIZE: f32 = 13.0;

/// 데이터 영역(표 본문·텍스트 모드) 배경. 순백 — 엑셀/EMEditor처럼 데이터가
/// 돋보이도록 UI 배경(옅은 회색 panel_fill)과 확실히 가른다.
pub fn data_bg() -> Color32 {
    Color32::WHITE
}

/// 줄무늬(짝수 행) 배경. 순백 위에 아주 옅게만 얹어 데이터를 가리지 않는다.
/// (egui 기본 striped는 회색이 짙어 순백 배경의 이점을 지운다.)
pub fn stripe_bg() -> Color32 {
    Color32::from_gray(250)
}

/// 셀 격자선 색(밝은 회색). 엑셀 기본 격자선과 비슷한 밝기.
pub fn grid_line() -> Color32 {
    Color32::from_gray(216)
}

/// 헤더 행 배경(데이터보다 한 톤 진한 회색).
pub fn header_bg() -> Color32 {
    Color32::from_gray(236)
}

/// 헤더 아래 구분선(격자선보다 진해 헤더와 데이터를 확실히 가른다).
pub fn header_rule() -> Color32 {
    Color32::from_gray(150)
}

/// 라인번호 컬럼 배경. 헤더보다 옅어 "데이터가 아닌 축"으로 읽힌다.
pub fn line_number_bg() -> Color32 {
    Color32::from_gray(244)
}

/// 라인번호 텍스트 색(본문보다 흐리게 — 데이터에서 눈을 뺏지 않는다).
pub fn line_number_fg() -> Color32 {
    Color32::from_gray(120)
}

/// Windows 표준 선택 파랑(#0078D7). 선택 음영/헤더 선택 배경의 기준색.
pub fn accent() -> Color32 {
    Color32::from_rgb(0, 120, 215)
}

/// 전체 매치 음영(옅은 보라). 순백 데이터 배경 위에 덧그리므로 알파를 낮게
/// 유지해 글자가 그대로 읽히게 한다 — 선택 음영(`sel_shade`, 알파 48)과 같은
/// 계열의 낮은 알파다. current(`find_current_bg`)보다 **확실히 옅어야** 두
/// 강조가 구분된다.
pub fn find_match_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(150, 90, 200, 64)
}

/// 현재 매치 음영(진한 보라). Find Next/Prev로 점프한 그 매치 하나만 이 색으로
/// 덮어 그려, 화면의 다른 옅은 매치들과 한눈에 구분되게 한다. `find_match_bg`
/// 위에 겹쳐 그리므로 그보다 확실히 진하다(알파 ~2배 + 채도 높음).
pub fn find_current_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(140, 55, 190, 140)
}

/// 스크롤 마커 거터의 매치 눈금(보라). 2px로 얇으므로 잘 보이도록 알파를 높게
/// 둔다. 여러 행이 한 픽셀에 겹치면 누적 알파로 진해져 밀집 구간이 드러난다
/// (의도된 동작 — S-7).
pub fn find_marker() -> Color32 {
    Color32::from_rgba_unmultiplied(150, 70, 200, 200)
}

/// 폰트 후보를 순서대로 시도해 처음 읽히는 것을 등록하고 그 키를 돌려준다.
/// 못 읽으면 `None`(해당 폰트는 건너뛴다 — 크래시 없음).
fn load_font(fonts: &mut FontDefinitions, key: &str, paths: &[&str]) -> Option<String> {
    let bytes = paths.iter().find_map(|p| std::fs::read(p).ok())?;
    fonts
        .font_data
        .insert(key.to_owned(), FontData::from_owned(bytes));
    Some(key.to_owned())
}

/// 데이터 영역은 고정폭(Cascadia Mono), UI는 Segoe UI, 한글은 맑은 고딕 폴백.
///
/// 폴백 체인 뒤에는 egui 내장 폰트를 남겨 둔다:
/// `"Hack"`(mono) / `"Ubuntu-Light"`(proportional)는 최후 글리프 폴백이고,
/// `"NotoEmoji-Regular"` / `"emoji-icon-font"`는 UI가 실제로 쓰는 기호
/// (`↑ ↓ ✖ ●`)를 그린다 — 빼면 그 글자들이 두부(□)가 된다.
/// (키 이름은 `epaint-0.28.1/src/text/fonts.rs:267-289` 확인.)
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let mono = load_font(
        &mut fonts,
        "mono",
        &[
            r"C:\Windows\Fonts\CascadiaMono.ttf",
            r"C:\Windows\Fonts\CascadiaCode.ttf",
            r"C:\Windows\Fonts\consola.ttf",
        ],
    );
    let ui = load_font(
        &mut fonts,
        "ui",
        &[
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
        ],
    );
    // 한글 폴백(영문 폰트에 한글 글리프가 없으므로 **뒤에** 붙인다).
    let kr = load_font(
        &mut fonts,
        "kr",
        &[
            r"C:\Windows\Fonts\malgun.ttf",
            r"C:\Windows\Fonts\gulim.ttc",
        ],
    );

    // Monospace: 고정폭 먼저, 한글은 뒤에서 폴백.
    let m = fonts.families.entry(FontFamily::Monospace).or_default();
    m.clear();
    if let Some(k) = &mono {
        m.push(k.clone());
    }
    if let Some(k) = &kr {
        m.push(k.clone());
    }
    m.push("Hack".to_owned());
    m.push("Ubuntu-Light".to_owned());
    m.push("NotoEmoji-Regular".to_owned());
    m.push("emoji-icon-font".to_owned());

    // Proportional: UI 폰트 먼저, 한글 폴백.
    let p = fonts.families.entry(FontFamily::Proportional).or_default();
    p.clear();
    if let Some(k) = &ui {
        p.push(k.clone());
    }
    if let Some(k) = &kr {
        p.push(k.clone());
    }
    p.push("Ubuntu-Light".to_owned());
    p.push("NotoEmoji-Regular".to_owned());
    p.push("emoji-icon-font".to_owned());

    ctx.set_fonts(fonts);
}

/// 역할별 텍스트 스타일. Windows 표준(Segoe UI 9pt ≈ 12~13px)에 맞춘다.
///
/// `Body`를 **Monospace로 두는 것이 핵심**이다: 표 셀과 텍스트 모드 줄은
/// `egui::Label`로 그려지고 `Label`은 `TextStyle::Body`를 쓴다
/// (`egui-0.28.1/src/widgets/label.rs`). 이 한 줄이 데이터 영역의 폰트를
/// 결정한다. 버튼/메뉴/체크박스는 `Button` 스타일이라 가변폭으로 남는다.
pub fn install_text_styles(ctx: &egui::Context) {
    ctx.style_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::new(MONO_SIZE, FontFamily::Monospace));
        s.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(MONO_SIZE, FontFamily::Monospace),
        );
        s.text_styles
            .insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        s.text_styles
            .insert(TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles
            .insert(TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
    });
}

/// 밝은 테마 + 각진 모서리 + Windows 파랑 선택. egui 기본은 둥근 모서리 +
/// 어두운 회색이라 "게임 툴" 인상이 나므로 Windows 앱풍으로 낮춘다.
pub fn install_visuals(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    // 모서리를 거의 각지게(Windows 앱은 둥글지 않다).
    let r = Rounding::same(2.0);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.rounding = r;
    v.widgets.active.rounding = r;
    v.widgets.open.rounding = r;
    v.window_rounding = Rounding::same(4.0);
    v.menu_rounding = Rounding::same(2.0);
    // 선택 강조는 Windows 파랑 계열로 통일.
    v.selection.bg_fill = accent();
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    // UI 영역(메뉴/툴바/상태바) 배경은 순백보다 살짝 회색이라야 데이터 영역과
    // 구분된다. 데이터 영역은 CentralPanel에 data_bg()를 따로 칠한다.
    v.panel_fill = Color32::from_gray(246);
    v.extreme_bg_color = data_bg();
    // 줄무늬는 격자선이 있으면 시끄러우므로 순백 위에 아주 옅게만.
    v.faint_bg_color = stripe_bg();
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        // 간격을 Windows 앱 수준으로 정돈(기본은 다소 헐렁하다).
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.menu_margin = egui::Margin::same(4.0);
        // `interact_size`는 egui 기본값(40×18)을 그대로 둔다 — 키우면 툴바가
        // 오히려 헐렁해지고, 줄이면 클릭 대상이 작아져 Windows 관례에 어긋난다.
    });
}

/// 앱 시작 시 한 번 호출 — 폰트·스타일·Visuals를 모두 설치한다.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    install_text_styles(ctx);
    install_visuals(ctx);
}

/// UI 크롬(툴바·상태바·다이얼로그)에서 쓰는 라벨 텍스트.
///
/// **왜 필요한가.** `TextStyle::Body`를 Monospace로 바꿨기 때문에
/// (데이터 영역이 고정폭이어야 하므로) 그냥 `ui.label("...")`을 쓰면 메뉴 옆
/// 안내 문구까지 고정폭으로 그려져 Windows 앱 인상이 깨진다. 크롬 텍스트는
/// 버튼과 같은 가변폭(`TextStyle::Button` = Proportional)이어야 한다.
///
/// `ui.label(chrome_text("..."))` 형태로 쓴다.
pub fn chrome_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).text_style(TextStyle::Button)
}

/// 창 제목. 파일이 열려 있으면 `"<파일명> — textViewer"`, 없으면 `"textViewer"`
/// (상용 에디터 관례). 경로 전체가 아니라 파일명만 쓴다 — 제목 표시줄은 짧다.
pub fn window_title(path: Option<&std::path::Path>) -> String {
    match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some(name) if !name.is_empty() => format!("{name} — textViewer"),
        _ => "textViewer".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn title_without_file_is_app_name() {
        assert_eq!(window_title(None), "textViewer");
    }

    #[test]
    fn title_with_file_shows_basename_only() {
        assert_eq!(
            window_title(Some(Path::new(r"C:\data\big.csv"))),
            "big.csv — textViewer"
        );
    }

    /// 디렉터리처럼 파일명이 없는 경로는 앱 이름으로 떨어져야 한다(크래시 없음).
    #[test]
    fn title_with_rootish_path_falls_back() {
        assert_eq!(window_title(Some(Path::new(r"C:\"))), "textViewer");
    }
}
