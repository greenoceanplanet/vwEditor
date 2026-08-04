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
//! - **UI 크롬은 가변폭**(Windows Segoe UI / macOS SF / Linux DejaVu Sans).
//!   메뉴·버튼·상태바.
//! - 폰트 파일이 없으면 그 단계만 건너뛰고 egui 내장 폰트로 떨어진다(크래시 없음).
//!
//! **폰트 파일은 시스템 것만 쓴다 — 앱에 폰트를 내장하지 않는다.** 실행 파일과
//! 저장소를 가볍게 유지하려는 선택이다. 대가는 한글 폰트가 없는 환경(주로
//! 최소 설치 리눅스)에서 한글이 두부(□)로 보인다는 것인데, 그 경우
//! `KOREAN_FONT_MISSING_MSG`로 설치 방법을 안내한다.

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

/// 데이터 영역 확대 배율의 허용 범위. Ctrl+휠이 이 범위 안에서만 움직인다.
///
/// 아래를 0.5로 막는 이유: 13px의 절반인 6.5px은 이미 판독 한계이고, 그 아래는
/// 행 높이가 글자보다 작아져 줄이 겹친다. 위를 4.0으로 막는 이유: 52px 글자에
/// 88px 행이면 화면에 열 줄이 안 들어와 "대용량 파일 뷰어"의 쓸모가 사라진다.
pub const MIN_VIEW_SCALE: f32 = 0.5;
pub const MAX_VIEW_SCALE: f32 = 4.0;

/// 배율을 적용한 데이터 영역 행 높이.
///
/// **폰트와 반드시 같은 배율이어야 한다.** 행 높이를 고정한 채 글자만 키우면
/// 큰 배율에서 글자가 행 밖으로 삐져나가 위아래가 잘리고, 반대로 글자만 줄이면
/// 행 사이가 허옇게 뜬다. 두 값이 한 배율에서 나와야 화면 비율이 유지된다.
pub fn row_height(scale: f32) -> f32 {
    ROW_HEIGHT * clamp_view_scale(scale)
}

/// 배율을 적용한 데이터 영역 고정폭 글자 크기.
pub fn mono_size(scale: f32) -> f32 {
    MONO_SIZE * clamp_view_scale(scale)
}

/// 배율을 허용 범위로 자른다. 저장된 설정이 깨졌거나(0, 음수, NaN) 범위를 벗어난
/// 값이 들어와도 화면이 무너지지 않게 하는 마지막 방어선이다 — NaN은 비교가 전부
/// 거짓이라 `clamp`가 그대로 통과시키므로 따로 걸러 1.0으로 돌린다.
pub fn clamp_view_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return 1.0;
    }
    scale.clamp(MIN_VIEW_SCALE, MAX_VIEW_SCALE)
}

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

/// 줄 끝 개행 기호(`␍␊`) 색. 파랑이되 본문 검정보다 눈에 띄면서도 데이터를
/// 가리지 않아야 한다.
///
/// `accent()`(#0078D7, 선택 음영과 같은 색)를 그대로 쓰지 않는 이유: 개행
/// 기호는 **모든 줄 끝에 하나씩** 늘 떠 있는 배경 정보라, 선택·헤더 강조와
/// 같은 색이면 "지금 선택된 것"과 "항상 있는 것"이 같은 무게로 보인다.
/// 한 단계 어둡고 채도를 낮춰(#3C78B4) 파랑임은 유지하되 뒤로 물러나게 한다.
pub fn line_ending_fg() -> Color32 {
    Color32::from_rgb(60, 120, 180)
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

/// 탭 문자가 차지하는 칸의 배경. 스페이스와 탭을 눈으로 가르기 위한 것이다
/// (둘 다 빈 칸으로 보여 구분이 안 된다).
///
/// **알파를 두 번 잘못 잡았다.** 그 과정이 이 값의 근거다.
///
/// 처음 28 → 순백 위에서 흰색과 10단계 남짓 차이라 아예 안 보였다.
/// 56으로 올렸지만 여전히 rgb(233,235,240)이라 "차이가 있는 것 같은데 눈으로
/// 구분이 안 가는" 수준이었다. 두 번 다 **머릿속 산술을 믿은 것**이 원인이다 —
/// `Color32`는 채널을 감마 공간에서 미리 곱해 들고 있고
/// (`ecolor-0.28.1/src/color32.rs:97-113`), GPU 합성도 선형 공간에서 일어나
/// 단순 비례 계산과 결과가 크게 다르다.
///
/// 120이면 순백 위에서 약 **rgb(204,210,222)** 로, 격자선(gray 216)보다 확실히
/// 진해 한눈에 보인다. 그러면서도 본문 글자(검정)와는 대비가 충분해 읽기를
/// 방해하지 않는다.
///
/// 참조값: `stripe_bg` gray(250) / `header_bg` gray(236) / `grid_line` gray(216).
/// 이 값을 바꾼다면 **실제 화면에서 확인할 것.** 계산으로는 두 번 틀렸다.
pub fn tab_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(110, 135, 175, 120)
}

/// 헥스 뷰 오프셋 컬럼 글자색. 본문과 구분되는 회청색.
pub fn hex_offset_fg() -> Color32 {
    Color32::from_rgb(120, 130, 145)
}

/// 헥스 찾기 매치 배경. 텍스트 하이라이트(`find_match_bg`)와 같은 계열이되
/// 함수를 분리해 둔다 — 헥스 패널은 순백 위 고정폭 두 자리라 텍스트 본문과
/// 대비 조건이 달라 조정 지점이 따로 필요하다.
///
/// `TextFormat::background`는 알파 합성 없이 그대로 칠해지므로(`find_match_bg`
/// 처럼 반투명을 쓰면 글자 뒤가 탁해진다) 불투명한 옅은 노랑을 쓴다.
pub fn hex_match_bg() -> Color32 {
    Color32::from_rgb(255, 235, 160)
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

/// 고정폭 폰트 후보. 데이터 영역용 — 숫자 자릿수가 세로로 맞아야 한다.
///
/// 플랫폼 순서가 아니라 **선호 순서**로 늘어놓는다. `load_font`가 처음 읽히는
/// 것을 쓰므로, 남의 플랫폼 경로는 그냥 읽기 실패로 건너뛰어진다. `cfg!` 분기를
/// 쓰지 않는 이유: 분기하면 각 플랫폼 목록이 그 플랫폼에서만 컴파일되어,
/// Windows에서 리눅스 경로 오타를 잡을 수 없다.
const MONO_CANDIDATES: &[&str] = &[
    // Windows
    r"C:\Windows\Fonts\CascadiaMono.ttf",
    r"C:\Windows\Fonts\CascadiaCode.ttf",
    r"C:\Windows\Fonts\consola.ttf",
    // macOS — SFNSMono는 시스템 UI 고정폭, Menlo는 터미널 기본.
    "/System/Library/Fonts/SFNSMono.ttf",
    "/System/Library/Fonts/Menlo.ttc",
    "/System/Library/Fonts/Monaco.dfont",
    // Linux — 배포판마다 경로가 다르므로 흔한 것을 넓게 깐다.
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
];

/// UI 크롬(메뉴·버튼·상태바)용 가변폭 폰트 후보.
const UI_CANDIDATES: &[&str] = &[
    // Windows
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\tahoma.ttf",
    // macOS
    "/System/Library/Fonts/SFNS.ttf",
    "/System/Library/Fonts/SFNSDisplay.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    // Linux
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
];

/// 한글 폴백 폰트 후보.
///
/// 이 목록이 **전부 실패하면 UI의 한글이 두부(□)가 된다** — egui 내장 폰트에는
/// CJK 글리프가 없다. 그래서 다른 둘과 달리 실패가 사용자에게 보고된다
/// (`FontReport::korean_missing`). 앱 자체는 정상 동작하므로 오류가 아니라
/// 안내다.
///
/// 리눅스 항목이 많은 이유: 배포판마다 한글 폰트 패키지 이름도 설치 경로도
/// 다르고, 최소 설치 이미지에는 CJK 폰트가 아예 없는 경우도 흔하다.
const KR_CANDIDATES: &[&str] = &[
    // Windows
    r"C:\Windows\Fonts\malgun.ttf",
    r"C:\Windows\Fonts\gulim.ttc",
    // macOS
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/Library/Fonts/AppleGothic.ttf",
    "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
    // Linux — 나눔(Ubuntu/Debian, Fedora), Noto CJK(Arch, 최신 배포판 공통).
    "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    "/usr/share/fonts/nhn-nanum/NanumGothic.ttf",
    "/usr/share/fonts/nanum/NanumGothic.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansKR-Regular.otf",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
];

/// 폰트 설치 결과. 화면에 한글이 그려질 수 있는지를 호출부에 알린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontReport {
    /// 한글 글리프를 가진 폰트를 하나도 못 찾았다. UI 한글이 두부가 된다.
    pub korean_missing: bool,
}

/// 한글 폰트를 못 찾았을 때 사용자에게 보일 안내.
///
/// 문구를 여기 두는 이유: 이 상황은 Windows 개발 환경에서 재현되지 않아
/// (맑은 고딕이 항상 있다) 테스트로만 검증된다. 테스트와 실제 문구가 같은
/// 상수를 봐야 어긋나지 않는다.
///
/// **이 문구만은 `i18n`을 타지 않고 영어로 고정한다.** 한글을 그릴 수 없다는
/// 사실을 알리는 안내가 한글이면 □로만 보여 아무것도 전달하지 못한다.
/// 언어 설정이 한국어여도 마찬가지다 — 설정이 아니라 폰트가 없는 것이다.
pub const KOREAN_FONT_MISSING_MSG: &str =
    "No Korean font was found, so Korean text appears as \u{25a1}. \
     Installing one fixes it — Debian/Ubuntu: `sudo apt install fonts-nanum`, \
     Fedora: `sudo dnf install nhn-nanum-fonts`, Arch: `sudo pacman -S noto-fonts-cjk`. \
     If a font is installed but still not found, add its path to KR_CANDIDATES in src/theme.rs.";

/// 데이터 영역은 고정폭(Cascadia Mono), UI는 Segoe UI, 한글은 맑은 고딕 폴백.
///
/// 폴백 체인 뒤에는 egui 내장 폰트를 남겨 둔다:
/// `"Hack"`(mono) / `"Ubuntu-Light"`(proportional)는 최후 글리프 폴백이고,
/// `"NotoEmoji-Regular"` / `"emoji-icon-font"`는 UI가 실제로 쓰는 기호
/// (`↑ ↓ ✖ ●`)를 그린다 — 빼면 그 글자들이 두부(□)가 된다.
/// (키 이름은 `epaint-0.28.1/src/text/fonts.rs:267-289` 확인.)
pub fn install_fonts(ctx: &egui::Context) -> FontReport {
    let mut fonts = FontDefinitions::default();

    let mono = load_font(&mut fonts, "mono", MONO_CANDIDATES);
    let ui = load_font(&mut fonts, "ui", UI_CANDIDATES);
    // 한글 폴백(영문 폰트에 한글 글리프가 없으므로 **뒤에** 붙인다).
    let kr = load_font(&mut fonts, "kr", KR_CANDIDATES);
    let report = FontReport {
        korean_missing: kr.is_none(),
    };

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
    report
}

/// 역할별 텍스트 스타일. Windows 표준(Segoe UI 9pt ≈ 12~13px)에 맞춘다.
///
/// `Body`를 **Monospace로 두는 것이 핵심**이다: 표 셀과 텍스트 모드 줄은
/// `egui::Label`로 그려지고 `Label`은 `TextStyle::Body`를 쓴다
/// (`egui-0.28.1/src/widgets/label.rs`). 이 한 줄이 데이터 영역의 폰트를
/// 결정한다. 버튼/메뉴/체크박스는 `Button` 스타일이라 가변폭으로 남는다.
/// **`scale`은 데이터 영역에만 적용된다.** `Body`/`Monospace`만 배율을 타고
/// `Button`/`Heading`/`Small`은 고정이다 — 메뉴·툴바·상태바는 Ctrl+휠로 확대해도
/// 그대로여야 한다는 것이 이 분리의 전부다. 배율이 바뀔 때마다 다시 부른다.
pub fn install_text_styles(ctx: &egui::Context, scale: f32) {
    let mono = mono_size(scale);
    ctx.style_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::new(mono, FontFamily::Monospace));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::new(mono, FontFamily::Monospace));
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
///
/// 폰트 설치 결과를 그대로 돌려준다. 호출부(`main`)가 이를 `App::start`에
/// 넘겨 한글 폰트 부재를 사용자에게 안내한다.
pub fn install(ctx: &egui::Context) -> FontReport {
    let report = install_fonts(ctx);
    install_text_styles(ctx, 1.0);
    install_visuals(ctx);
    report
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

/// 창 제목. 파일이 열려 있으면 `"<파일명> — vwEditor"`, 없으면 `"vwEditor"`
/// (상용 에디터 관례). 경로 전체가 아니라 파일명만 쓴다 — 제목 표시줄은 짧다.
pub fn window_title(path: Option<&std::path::Path>) -> String {
    match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some(name) if !name.is_empty() => format!("{name} — vwEditor"),
        _ => "vwEditor".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ---- 폰트 후보 목록 ----
    //
    // 목록의 대부분은 **남의 플랫폼 경로**라 이 환경에서 읽어 볼 수 없다.
    // 그래서 "존재하는가"가 아니라 "형태가 맞는가"를 본다 — 오타·중복·빈 항목
    // 같은, 실제로 저지르는 실수를 잡는 게 목적이다.

    /// 세 목록 모두 세 플랫폼을 실제로 담고 있다. 한 플랫폼이 통째로 빠지면
    /// 그 OS에서 폰트가 하나도 안 잡힌다 — 목록을 손볼 때 가장 쉬운 실수다.
    #[test]
    fn font_candidates_cover_all_three_platforms() {
        for (name, list) in [
            ("mono", MONO_CANDIDATES),
            ("ui", UI_CANDIDATES),
            ("kr", KR_CANDIDATES),
        ] {
            assert!(
                list.iter().any(|p| p.starts_with(r"C:\Windows")),
                "{name}: Windows 경로가 없다"
            );
            assert!(
                list.iter().any(|p| p.starts_with("/System/") || p.starts_with("/Library/")),
                "{name}: macOS 경로가 없다"
            );
            assert!(
                list.iter().any(|p| p.starts_with("/usr/share/fonts")),
                "{name}: Linux 경로가 없다"
            );
        }
    }

    /// 절대 경로여야 하고 빈 항목이 없어야 한다. 상대 경로는 실행 위치에 따라
    /// 결과가 달라져(테스트에서만 우연히 통과하는) 재현 불가능한 버그가 된다.
    #[test]
    fn font_candidates_are_absolute_and_nonempty() {
        for (name, list) in [
            ("mono", MONO_CANDIDATES),
            ("ui", UI_CANDIDATES),
            ("kr", KR_CANDIDATES),
        ] {
            assert!(!list.is_empty(), "{name}: 목록이 비었다");
            for p in list {
                assert!(!p.trim().is_empty(), "{name}: 빈 경로 항목");
                let absolute = p.starts_with('/') || p.starts_with(r"C:\");
                assert!(absolute, "{name}: 절대 경로가 아니다 — {p}");
            }
        }
    }

    /// 같은 경로를 두 번 적지 않는다. 중복은 무해해 보이지만, 목록을 손볼 때
    /// 한쪽만 고치고 다른 쪽을 놓치는 함정이 된다.
    #[test]
    fn font_candidates_have_no_duplicates() {
        for (name, list) in [
            ("mono", MONO_CANDIDATES),
            ("ui", UI_CANDIDATES),
            ("kr", KR_CANDIDATES),
        ] {
            let mut seen = std::collections::HashSet::new();
            for p in list {
                assert!(seen.insert(*p), "{name}: 중복 경로 — {p}");
            }
        }
    }

    /// 안내 문구는 "무엇이 문제인지"와 "어떻게 고치는지"를 둘 다 담아야 한다.
    /// 증상만 알리고 해결책이 없으면 사용자는 앱을 지운다.
    #[test]
    fn korean_font_message_tells_how_to_fix_it() {
        let m = KOREAN_FONT_MISSING_MSG;
        assert!(m.contains("Korean font"), "증상을 설명한다: {m}");
        assert!(
            m.contains("apt") && m.contains("dnf") && m.contains("pacman"),
            "주요 배포판 설치 명령을 안내한다: {m}"
        );
        assert!(
            m.contains("KR_CANDIDATES"),
            "목록에 없는 폰트를 쓰는 사용자를 위해 고칠 지점을 알려준다"
        );
    }

    /// 이 안내만은 반드시 **영어**여야 한다 — 한글을 그릴 수 없다는 사실을
    /// 알리는 문구가 한글이면 □로만 보인다. i18n을 타지 않는 이유이기도 하다.
    #[test]
    fn korean_font_message_is_ascii_only() {
        let m = KOREAN_FONT_MISSING_MSG;
        let hangul: Vec<char> = m.chars().filter(|c| ('가'..='힣').contains(c)).collect();
        assert!(
            hangul.is_empty(),
            "한글 폰트가 없을 때 보일 문구에 한글이 있다: {hangul:?}"
        );
    }

    /// 이 환경(Windows)에서는 한글 폰트가 반드시 잡힌다. 이 테스트가 깨지면
    /// Windows 경로 목록이 망가진 것이다 — 실제 파일을 읽는 유일한 검증이다.
    #[test]
    fn windows_korean_font_actually_loads_here() {
        if !cfg!(target_os = "windows") {
            return; // 다른 OS에서는 확인할 수 없다.
        }
        let found = KR_CANDIDATES
            .iter()
            .filter(|p| p.starts_with(r"C:\Windows"))
            .any(|p| std::fs::metadata(p).is_ok());
        assert!(found, "Windows에 한글 폰트가 하나는 있어야 한다: {KR_CANDIDATES:?}");
    }

    #[test]
    fn title_without_file_is_app_name() {
        assert_eq!(window_title(None), "vwEditor");
    }

    #[test]
    fn title_with_file_shows_basename_only() {
        assert_eq!(
            window_title(Some(Path::new(r"C:\data\big.csv"))),
            "big.csv — vwEditor"
        );
    }

    /// 디렉터리처럼 파일명이 없는 경로는 앱 이름으로 떨어져야 한다(크래시 없음).
    #[test]
    fn title_with_rootish_path_falls_back() {
        assert_eq!(window_title(Some(Path::new(r"C:\"))), "vwEditor");
    }
}
