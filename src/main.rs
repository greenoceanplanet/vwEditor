// 릴리스 빌드에서는 콘솔 창을 띄우지 않는다(GUI 앱).
// Rust 기본은 콘솔 서브시스템이라 exe를 실행하면 cmd 창이 함께 뜬다.
// 디버그 빌드에서는 println!/패닉 메시지를 봐야 하므로 콘솔을 남긴다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 전역 할당자를 Windows 기본 힙 대신 mimalloc으로 바꾼다.
///
/// 이 앱은 파일 한 줄당 `String` 하나를 쥔다 — 15.4M행이면 할당이 1,540만 개다.
/// 그 규모에서 `RtlAllocateHeap`의 락·블록 헤더·흩어진 배치가 지배적인 비용이
/// 된다. 실파일(899MB, 15.4M행) 측정 — 전체 바꾸기 **2,863ms → 1,193ms(2.4배)**,
/// 정렬의 재배치 **3,346ms → 785ms(4.3배)**, 파일 열기 1,347ms → 1,050ms.
///
/// 이득이 치환에만 있는 게 아니다. 파일 열기·정렬·저장·찾기가 전부 할당 집약적이라
/// 같이 빨라지고, **할당을 하나도 안 하는 판정 패스조차** 2배 이상 빨라진다 —
/// mimalloc이 줄 String들을 훨씬 촘촘히 모아 놓아 스캔이 건드리는 캐시 라인과
/// 페이지 수가 줄기 때문이다.
///
/// jemalloc이 아닌 이유: jemalloc은 MSVC 타깃을 지원하지 않는다. Windows에서
/// 실용적인 선택지는 mimalloc뿐이다.
///
/// 메모리 대가는 실측했다 — mimalloc이 30~60% 더 커밋한다는 보고가 있어
/// 확인한 것인데, 이 워크로드에는 해당하지 않는다. 같은 파일에서 상주
/// 메모리가 치환 직후 3,343MB → 3,235MB, 피크 4,881MB → 4,602MB로 **오히려
/// 조금 줄었다**. 줄당 String이 작고(평균 58B) 균일해 mimalloc의 크기별
/// 슬랩이 Windows 힙의 블록 헤더보다 덜 낭비하기 때문으로 보인다.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod app;
mod edit;
mod find;
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
