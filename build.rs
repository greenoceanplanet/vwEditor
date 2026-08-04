//! exe에 아이콘·버전 정보를 심는다(Windows 전용).
//!
//! 아이콘 원본은 `assets/vweditor.ico` — `scripts/gen_icon.py`가 만든다.
//! 그림을 고치려면 그 스크립트를 고쳐 다시 돌린다(손으로 .ico를 편집하지 않는다).
fn main() {
    // 다른 OS에서는 winres가 아무것도 하지 않지만, 빌드 스크립트가 통째로
    // 도는 것 자체를 막아 크로스 컴파일 시 불필요한 경고를 없앤다.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/vweditor.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/vweditor.ico");
        if let Err(e) = res.compile() {
            // 리소스 컴파일 실패로 빌드를 통째로 막지 않는다 — 아이콘은
            // 없어도 프로그램은 동작한다.
            println!("cargo:warning=아이콘 임베드 실패: {e}");
        }
    }
}
