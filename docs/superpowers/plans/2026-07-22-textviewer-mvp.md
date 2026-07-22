# textViewer MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** EMEditor처럼 10GB+ CSV/TSV/텍스트 파일을 여는 순간 즉시 테이블로 보여주는 단일 .exe Windows 뷰어를 만든다.

**Architecture:** 4개 모듈(`source`=mmap, `index`=줄 offset+인덱싱 상태, `parse`=인코딩 디코딩+필드 분리, `app`=egui UI). 파일을 memmap2로 매핑하고 백그라운드 스레드가 개행 offset을 점진적으로 인덱싱한다. UI는 매 프레임 "보이는 행 + 버퍼"만 offset으로 조회·디코딩·파싱해 egui 가상 스크롤 테이블로 렌더한다.

**Tech Stack:** Rust, eframe/egui, egui_extras(TableBuilder), memmap2, csv-core, encoding_rs, rfd(파일 다이얼로그), std::thread + Arc/Atomic.

## Global Constraints

- 플랫폼: Windows, 단일 `.exe` (`cargo build --release` → `target/release/textviewer.exe`). 설치 불필요.
- Rust edition 2021.
- 파일 크기와 무관하게 메모리 낮게: 파일은 mmap, 상주 인덱스는 `Vec<u64>`(줄당 8바이트)만.
- 첫 화면 표시 목표 0.5초 이내(1단계는 앞부분만 스캔).
- 인코딩: UTF-8 / CP949(EUC-KR) / UTF-16LE / UTF-16BE 만. 감지 실패 fallback = CP949.
- 개행 offset 인덱싱은 바이트 레벨. UTF-8/CP949는 `0x0A`, UTF-16LE는 `0A 00`, UTF-16BE는 `00 0A` 패턴.
- MVP 제외: 찾기/바꾸기, 편집·저장, 정렬·필터, 외부 변경 감지.
- 크래시 금지: 잘못된 바이트·긴 줄·필드 수 불일치는 손실 없이 표시하고 계속.

---

## File Structure

```
textViewer/
├── Cargo.toml
├── src/
│   ├── main.rs        — eframe 진입점, App 실행
│   ├── source.rs      — memmap2 파일 매핑, byte 슬라이스 제공
│   ├── index.rs       — 줄 offset 저장소 + 인덱싱 상태(Arc 공유)
│   ├── indexer.rs     — 백그라운드 인덱싱 워커 (개행 스캔, 중단/이어서)
│   ├── parse.rs       — 인코딩/구분자/헤더 감지 + 행 분리(순수 함수)
│   └── app.rs         — egui UI (툴바, 테이블, 상태바), 상태 소유
└── tests/
    └── (단위 테스트는 각 모듈 내 #[cfg(test)] 모듈로)
```

- `parse.rs`는 순수 함수만 — I/O·스레드 없음. 단위테스트 쉬움. 먼저 만든다.
- `source.rs`는 mmap만. `index.rs`는 공유 상태 자료구조만. `indexer.rs`가 이 둘을 연결하는 스레드 로직.
- `app.rs`가 전부 조합. main.rs는 얇게.

---

## Task 1: 프로젝트 스캐폴딩 + 빌드 확인

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

**Interfaces:**
- Consumes: 없음
- Produces: 빌드 가능한 빈 eframe 앱. `App` 구조체는 Task 8에서 채운다.

- [ ] **Step 1: Cargo.toml 작성**

```toml
[package]
name = "textviewer"
version = "0.1.0"
edition = "2021"

[dependencies]
eframe = "0.28"
egui = "0.28"
egui_extras = "0.28"
memmap2 = "0.9"
csv-core = "0.1"
encoding_rs = "0.8"
rfd = "0.14"

[profile.release]
opt-level = 3
lto = true
```

- [ ] **Step 2: 최소 main.rs 작성**

```rust
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
```

- [ ] **Step 3: 빌드 확인**

Run: `cargo build`
Expected: 성공 (의존성 다운로드 후 컴파일 완료, 에러 없음)

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "chore: scaffold eframe app"
```

---

## Task 2: 인코딩 감지 (parse.rs)

**Files:**
- Create: `src/parse.rs`
- Modify: `src/main.rs` (add `mod parse;`)

**Interfaces:**
- Consumes: 없음
- Produces:
  - `pub enum Encoding { Utf8, Cp949, Utf16Le, Utf16Be }`
  - `pub fn detect_encoding(head: &[u8]) -> Encoding`

- [ ] **Step 1: 실패하는 테스트 작성**

`src/parse.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_utf8_bom() {
        let bytes = [0xEF, 0xBB, 0xBF, b'a', b'b'];
        assert_eq!(detect_encoding(&bytes), Encoding::Utf8);
    }

    #[test]
    fn detects_utf16le_bom() {
        let bytes = [0xFF, 0xFE, b'a', 0x00];
        assert_eq!(detect_encoding(&bytes), Encoding::Utf16Le);
    }

    #[test]
    fn detects_utf16be_bom() {
        let bytes = [0xFE, 0xFF, 0x00, b'a'];
        assert_eq!(detect_encoding(&bytes), Encoding::Utf16Be);
    }

    #[test]
    fn plain_ascii_is_utf8() {
        assert_eq!(detect_encoding(b"name,age\n"), Encoding::Utf8);
    }

    #[test]
    fn valid_utf8_korean_is_utf8() {
        // "이름" in UTF-8
        assert_eq!(detect_encoding("이름".as_bytes()), Encoding::Utf8);
    }

    #[test]
    fn invalid_utf8_falls_back_to_cp949() {
        // "가" in CP949 = 0xB0 0xA1, which is NOT valid UTF-8
        let bytes = [0xB0, 0xA1];
        assert_eq!(detect_encoding(&bytes), Encoding::Cp949);
    }
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test parse::tests::detects_utf8_bom`
Expected: FAIL — `Encoding` / `detect_encoding` not found

- [ ] **Step 3: 최소 구현 작성**

`src/parse.rs` 맨 위:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Cp949,
    Utf16Le,
    Utf16Be,
}

/// 파일 앞부분 바이트로 인코딩을 감지한다.
/// 순서: BOM → UTF-8 유효성 → CP949 fallback.
pub fn detect_encoding(head: &[u8]) -> Encoding {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Encoding::Utf8;
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return Encoding::Utf16Le;
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return Encoding::Utf16Be;
    }
    if std::str::from_utf8(head).is_ok() {
        return Encoding::Utf8;
    }
    // 앞부분이 멀티바이트 문자 중간에서 잘렸을 수 있으니, 마지막 몇 바이트를 잘라 재검사.
    for cut in 1..=3.min(head.len()) {
        if std::str::from_utf8(&head[..head.len() - cut]).is_ok() {
            return Encoding::Utf8;
        }
    }
    Encoding::Cp949
}
```

Modify `src/main.rs`: 파일 맨 위에 `mod parse;` 추가.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test parse::tests`
Expected: 6개 테스트 모두 PASS

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs src/main.rs
git commit -m "feat: encoding detection (utf-8/cp949/utf-16)"
```

---

## Task 3: 바이트→문자열 디코딩 (parse.rs)

**Files:**
- Modify: `src/parse.rs`

**Interfaces:**
- Consumes: `Encoding` (Task 2)
- Produces: `pub fn decode_line(bytes: &[u8], enc: Encoding) -> String`

- [ ] **Step 1: 실패하는 테스트 작성**

`src/parse.rs` tests 모듈에 추가:
```rust
    #[test]
    fn decode_utf8_line() {
        assert_eq!(decode_line("이름,나이".as_bytes(), Encoding::Utf8), "이름,나이");
    }

    #[test]
    fn decode_cp949_line() {
        // "가나" in CP949
        let bytes = [0xB0, 0xA1, 0xB3, 0xAA];
        assert_eq!(decode_line(&bytes, Encoding::Cp949), "가나");
    }

    #[test]
    fn decode_invalid_bytes_no_panic() {
        // 잘못된 바이트도 패닉 없이 대체문자로 표시
        let bytes = [0xFF, 0xFE, 0x00];
        let s = decode_line(&bytes, Encoding::Utf8);
        assert!(!s.is_empty());
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test parse::tests::decode_cp949_line`
Expected: FAIL — `decode_line` not found

- [ ] **Step 3: 최소 구현 작성**

`src/parse.rs`에 추가:
```rust
use encoding_rs::{EUC_KR, UTF_16BE, UTF_16LE, UTF_8};

/// 한 줄(개행 제외 권장)의 바이트를 인코딩에 맞춰 문자열로 디코딩한다.
/// 잘못된 바이트는 대체문자(U+FFFD)로 손실 없이 처리한다(패닉 없음).
pub fn decode_line(bytes: &[u8], enc: Encoding) -> String {
    let encoding = match enc {
        Encoding::Utf8 => UTF_8,
        Encoding::Cp949 => EUC_KR,
        Encoding::Utf16Le => UTF_16LE,
        Encoding::Utf16Be => UTF_16BE,
    };
    let (cow, _used, _had_errors) = encoding.decode(bytes);
    cow.into_owned()
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test parse::tests`
Expected: 모든 테스트 PASS (기존 6 + 신규 3)

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs
git commit -m "feat: decode line bytes to string per encoding"
```

---

## Task 4: 구분자 감지 + 행 분리 (parse.rs)

**Files:**
- Modify: `src/parse.rs`

**Interfaces:**
- Consumes: 없음(디코딩된 `&str`에 대해 동작)
- Produces:
  - `pub fn detect_delimiter(path: &std::path::Path, head_lines: &[&str]) -> u8`
  - `pub fn split_fields(line: &str, delim: u8) -> Vec<String>`

- [ ] **Step 1: 실패하는 테스트 작성**

`src/parse.rs` tests 모듈에 추가:
```rust
    use std::path::Path;

    #[test]
    fn tsv_extension_picks_tab() {
        assert_eq!(detect_delimiter(Path::new("a.tsv"), &["x\ty"]), b'\t');
    }

    #[test]
    fn csv_extension_picks_comma() {
        assert_eq!(detect_delimiter(Path::new("a.csv"), &["x,y"]), b',');
    }

    #[test]
    fn txt_content_picks_consistent_delimiter() {
        // 매 줄 파이프가 정확히 2개로 일관 → 파이프
        let lines = ["a|b|c", "d|e|f", "g|h|i"];
        assert_eq!(detect_delimiter(Path::new("a.txt"), &lines), b'|');
    }

    #[test]
    fn split_basic_fields() {
        assert_eq!(split_fields("a,b,c", b','), vec!["a", "b", "c"]);
    }

    #[test]
    fn split_quoted_field_with_delimiter() {
        // "a,b" 는 한 필드
        assert_eq!(split_fields("\"a,b\",c", b','), vec!["a,b", "c"]);
    }

    #[test]
    fn split_empty_fields() {
        assert_eq!(split_fields("a,,c", b','), vec!["a", "", "c"]);
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test parse::tests::split_basic_fields`
Expected: FAIL — `split_fields` not found

- [ ] **Step 3: 최소 구현 작성**

`src/parse.rs`에 추가:
```rust
const DELIMITER_CANDIDATES: [u8; 4] = [b',', b'\t', b'|', b';'];

/// 확장자 우선, 애매하면 앞부분 여러 줄에서 가장 일관된 구분자 선택.
pub fn detect_delimiter(path: &std::path::Path, head_lines: &[&str]) -> u8 {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "tsv" | "tab" => return b'\t',
            "csv" => return b',',
            "psv" => return b'|',
            _ => {}
        }
    }
    // 내용 분석: 각 후보의 줄당 등장 횟수 분산이 가장 낮고 평균이 1 이상인 것.
    let mut best = b',';
    let mut best_score = f64::MAX;
    for &cand in &DELIMITER_CANDIDATES {
        let counts: Vec<usize> = head_lines
            .iter()
            .map(|l| l.bytes().filter(|&b| b == cand).count())
            .collect();
        if counts.is_empty() {
            continue;
        }
        let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        if mean < 1.0 {
            continue; // 이 구분자는 거의 안 나옴
        }
        let variance = counts
            .iter()
            .map(|&c| (c as f64 - mean).powi(2))
            .sum::<f64>()
            / counts.len() as f64;
        if variance < best_score {
            best_score = variance;
            best = cand;
        }
    }
    best
}

/// 한 줄을 구분자로 분리. 따옴표(")로 감싼 필드 안의 구분자는 무시.
pub fn split_fields(line: &str, delim: u8) -> Vec<String> {
    let mut reader = csv_core::ReaderBuilder::new()
        .delimiter(delim)
        .build();
    let bytes = line.as_bytes();
    let mut fields = Vec::new();
    let mut input = bytes;
    // 넉넉한 출력 버퍼
    let mut out = vec![0u8; bytes.len() + 1];
    let mut ends = vec![0usize; 64];
    loop {
        let (result, nin, nout, nend) =
            reader.read_record(input, &mut out, &mut ends);
        for i in 0..nend {
            let start = if i == 0 { 0 } else { ends[i - 1] };
            let end = ends[i];
            fields.push(String::from_utf8_lossy(&out[start..end]).into_owned());
        }
        input = &input[nin..];
        let _ = nout;
        match result {
            csv_core::ReadRecordResult::End
            | csv_core::ReadRecordResult::InputEmpty => break,
            csv_core::ReadRecordResult::OutputFull => {
                out.resize(out.len() * 2, 0);
            }
            csv_core::ReadRecordResult::OutputEndsFull => {
                ends.resize(ends.len() * 2, 0);
            }
            csv_core::ReadRecordResult::Record => {
                // 한 레코드 완성 — 단일 줄 입력이므로 계속 루프하면 End 도달
            }
        }
    }
    if fields.is_empty() {
        fields.push(String::new());
    }
    fields
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test parse::tests`
Expected: 모든 테스트 PASS

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs
git commit -m "feat: delimiter detection and quoted field splitting"
```

---

## Task 5: 헤더 감지 (parse.rs)

**Files:**
- Modify: `src/parse.rs`

**Interfaces:**
- Consumes: `split_fields` (Task 4)
- Produces: `pub fn detect_header(rows: &[Vec<String>]) -> bool`

- [ ] **Step 1: 실패하는 테스트 작성**

`src/parse.rs` tests 모듈에 추가:
```rust
    #[test]
    fn header_when_first_row_text_rest_numeric() {
        let rows = vec![
            vec!["name".to_string(), "age".to_string()],
            vec!["Alice".to_string(), "30".to_string()],
            vec!["Bob".to_string(), "25".to_string()],
        ];
        assert!(detect_header(&rows));
    }

    #[test]
    fn no_header_when_all_numeric() {
        let rows = vec![
            vec!["1".to_string(), "2".to_string()],
            vec!["3".to_string(), "4".to_string()],
        ];
        assert!(!detect_header(&rows));
    }

    #[test]
    fn header_on_when_ambiguous_all_text() {
        // 전부 문자열이면 애매 → 안전하게 헤더 ON
        let rows = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string(), "d".to_string()],
        ];
        assert!(detect_header(&rows));
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test parse::tests::header_when_first_row_text_rest_numeric`
Expected: FAIL — `detect_header` not found

- [ ] **Step 3: 최소 구현 작성**

`src/parse.rs`에 추가:
```rust
/// 첫 줄이 헤더인지 추정.
/// - 첫 줄 전부 비수치 && 아래 줄들에 수치 필드 존재 → 헤더
/// - 애매하면(첫 줄과 아래 타입이 유사) → 안전하게 true(헤더 ON)
pub fn detect_header(rows: &[Vec<String>]) -> bool {
    if rows.len() < 2 {
        return true; // 판단 근거 부족 → 안전 기본값
    }
    let is_numeric = |s: &str| s.trim().parse::<f64>().is_ok();

    let first = &rows[0];
    let first_all_text = !first.is_empty() && first.iter().all(|f| !is_numeric(f));

    let body_has_numeric = rows[1..]
        .iter()
        .any(|r| r.iter().any(|f| is_numeric(f)));

    if first_all_text && body_has_numeric {
        return true;
    }
    // 첫 줄에 수치가 섞여 있고 아래도 비슷하면 데이터일 가능성 → 헤더 아님
    let first_has_numeric = first.iter().any(|f| is_numeric(f));
    if first_has_numeric && body_has_numeric {
        return false;
    }
    // 그 외(전부 텍스트 등 애매) → 헤더 ON
    true
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test parse::tests`
Expected: 모든 테스트 PASS

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs
git commit -m "feat: header row detection"
```

---

## Task 6: mmap 파일 소스 (source.rs)

**Files:**
- Create: `src/source.rs`
- Modify: `src/main.rs` (add `mod source;`)

**Interfaces:**
- Consumes: 없음
- Produces:
  - `pub struct Source { mmap: memmap2::Mmap }`
  - `pub fn open(path: &std::path::Path) -> std::io::Result<Source>`
  - `impl Source { pub fn len(&self) -> u64; pub fn slice(&self, start: u64, end: u64) -> &[u8]; pub fn as_bytes(&self) -> &[u8]; }`

- [ ] **Step 1: 실패하는 테스트 작성**

`src/source.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file_with(content: &[u8]) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("tv_test_{}.txt", content.len()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn opens_and_reports_len() {
        let p = temp_file_with(b"hello world");
        let src = open(&p).unwrap();
        assert_eq!(src.len(), 11);
    }

    #[test]
    fn slice_returns_correct_bytes() {
        let p = temp_file_with(b"hello world");
        let src = open(&p).unwrap();
        assert_eq!(src.slice(0, 5), b"hello");
        assert_eq!(src.slice(6, 11), b"world");
    }

    #[test]
    fn missing_file_returns_err() {
        assert!(open(std::path::Path::new("no_such_file_xyz.txt")).is_err());
    }
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test source::tests::opens_and_reports_len`
Expected: FAIL — `open` not found

- [ ] **Step 3: 최소 구현 작성**

`src/source.rs` 맨 위:
```rust
use memmap2::Mmap;
use std::path::Path;

/// mmap으로 매핑된 읽기 전용 파일. 파일 크기와 무관하게 즉시 매핑된다.
pub struct Source {
    mmap: Mmap,
}

/// 파일을 열어 메모리 매핑한다. 빈 파일도 허용.
pub fn open(path: &Path) -> std::io::Result<Source> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if meta.len() == 0 {
        // 빈 파일은 mmap이 실패할 수 있으니 빈 매핑을 흉내내기 위해 임시 처리.
        // memmap2는 길이 0 매핑을 허용하지 않으므로, 별도 플래그 대신 1바이트 파일로 취급하지 않고
        // 안전하게 처리: 빈 Mmap을 만들 수 없으므로 여기서는 에러 대신 아래처럼 처리한다.
        // 실전에서는 빈 매핑을 위해 anonymous map을 쓴다.
        let map = memmap2::MmapOptions::new().len(0).map_anon()?.make_read_only()?;
        return Ok(Source { mmap: map });
    }
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(Source { mmap })
}

impl Source {
    pub fn len(&self) -> u64 {
        self.mmap.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.mmap.is_empty()
    }

    /// [start, end) 범위의 바이트 슬라이스. 범위는 파일 크기로 클램프된다.
    pub fn slice(&self, start: u64, end: u64) -> &[u8] {
        let len = self.mmap.len() as u64;
        let s = start.min(len) as usize;
        let e = end.min(len).max(start) as usize;
        &self.mmap[s..e]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap[..]
    }
}
```

Modify `src/main.rs`: `mod source;` 추가.

> 주: `map_anon().make_read_only()` 조합이 컴파일 안 되면, 빈 파일 처리는 `Source`에 `empty: bool` 필드를 두고 `slice`가 항상 `&[]`를 반환하도록 대체한다. 우선 위 구현으로 빌드 시도.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test source::tests`
Expected: 3개 테스트 PASS

- [ ] **Step 5: Commit**

```bash
git add src/source.rs src/main.rs
git commit -m "feat: mmap file source with byte slicing"
```

---

## Task 7: 줄 인덱스 + 인덱싱 상태 (index.rs)

**Files:**
- Create: `src/index.rs`
- Modify: `src/main.rs` (add `mod index;`)

**Interfaces:**
- Consumes: 없음
- Produces:
  - `pub enum Phase { Priming, Indexing, Paused, Complete }`
  - `pub struct IndexStatus { pub phase: Phase, pub bytes_done: u64, pub total_bytes: u64 }`
  - `pub struct LineIndex` — 공유 자료구조. 아래 메서드 제공:
    - `pub fn new(total_bytes: u64) -> LineIndex`
    - `pub fn push_offset(&self, off: u64)`
    - `pub fn line_count(&self) -> usize`
    - `pub fn offset(&self, row: usize) -> Option<u64>`
    - `pub fn line_range(&self, row: usize) -> Option<(u64, u64)>` (해당 행의 byte 시작~끝)
    - `pub fn set_phase(&self, phase: Phase)`, `pub fn set_bytes_done(&self, n: u64)`
    - `pub fn status(&self) -> IndexStatus`
    - `pub fn request_pause(&self)`, `pub fn pause_requested(&self) -> bool`, `pub fn clear_pause(&self)`
  - 내부 공유는 `Arc<Inner>`, `LineIndex`는 `Clone`(Arc clone).

- [ ] **Step 1: 실패하는 테스트 작성**

`src/index.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_count() {
        let idx = LineIndex::new(100);
        idx.push_offset(0);
        idx.push_offset(10);
        idx.push_offset(20);
        assert_eq!(idx.line_count(), 3);
    }

    #[test]
    fn line_range_uses_next_offset() {
        let idx = LineIndex::new(100);
        idx.push_offset(0);
        idx.push_offset(10);
        idx.push_offset(25);
        // 0행: [0,10), 1행: [10,25)
        assert_eq!(idx.line_range(0), Some((0, 10)));
        assert_eq!(idx.line_range(1), Some((10, 25)));
    }

    #[test]
    fn last_line_range_uses_total_bytes() {
        let idx = LineIndex::new(30);
        idx.push_offset(0);
        idx.push_offset(10);
        // 마지막 행(1)은 다음 offset이 없으니 total_bytes(30)까지
        assert_eq!(idx.line_range(1), Some((10, 30)));
    }

    #[test]
    fn pause_flag_roundtrip() {
        let idx = LineIndex::new(0);
        assert!(!idx.pause_requested());
        idx.request_pause();
        assert!(idx.pause_requested());
        idx.clear_pause();
        assert!(!idx.pause_requested());
    }

    #[test]
    fn status_reflects_setters() {
        let idx = LineIndex::new(1000);
        idx.set_phase(Phase::Indexing);
        idx.set_bytes_done(400);
        let st = idx.status();
        assert_eq!(st.phase, Phase::Indexing);
        assert_eq!(st.bytes_done, 400);
        assert_eq!(st.total_bytes, 1000);
    }
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test index::tests::push_and_count`
Expected: FAIL — `LineIndex` not found

- [ ] **Step 3: 최소 구현 작성**

`src/index.rs` 맨 위:
```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Priming,
    Indexing,
    Paused,
    Complete,
}

#[derive(Debug, Clone, Copy)]
pub struct IndexStatus {
    pub phase: Phase,
    pub bytes_done: u64,
    pub total_bytes: u64,
}

struct Inner {
    offsets: RwLock<Vec<u64>>,
    total_bytes: u64,
    bytes_done: AtomicU64,
    phase: RwLock<Phase>,
    pause: AtomicBool,
}

/// 줄 시작 offset 배열 + 인덱싱 상태. Arc 공유(Clone은 Arc clone).
#[derive(Clone)]
pub struct LineIndex {
    inner: Arc<Inner>,
}

impl LineIndex {
    pub fn new(total_bytes: u64) -> LineIndex {
        LineIndex {
            inner: Arc::new(Inner {
                offsets: RwLock::new(Vec::new()),
                total_bytes,
                bytes_done: AtomicU64::new(0),
                phase: RwLock::new(Phase::Priming),
                pause: AtomicBool::new(false),
            }),
        }
    }

    pub fn push_offset(&self, off: u64) {
        self.inner.offsets.write().unwrap().push(off);
    }

    pub fn line_count(&self) -> usize {
        self.inner.offsets.read().unwrap().len()
    }

    pub fn offset(&self, row: usize) -> Option<u64> {
        self.inner.offsets.read().unwrap().get(row).copied()
    }

    /// 해당 행의 byte 범위 [start, end). 마지막 행이면 end = total_bytes.
    pub fn line_range(&self, row: usize) -> Option<(u64, u64)> {
        let offsets = self.inner.offsets.read().unwrap();
        let start = *offsets.get(row)?;
        let end = offsets
            .get(row + 1)
            .copied()
            .unwrap_or(self.inner.total_bytes);
        Some((start, end))
    }

    pub fn set_phase(&self, phase: Phase) {
        *self.inner.phase.write().unwrap() = phase;
    }

    pub fn set_bytes_done(&self, n: u64) {
        self.inner.bytes_done.store(n, Ordering::Relaxed);
    }

    pub fn status(&self) -> IndexStatus {
        IndexStatus {
            phase: *self.inner.phase.read().unwrap(),
            bytes_done: self.inner.bytes_done.load(Ordering::Relaxed),
            total_bytes: self.inner.total_bytes,
        }
    }

    pub fn request_pause(&self) {
        self.inner.pause.store(true, Ordering::Relaxed);
    }

    pub fn pause_requested(&self) -> bool {
        self.inner.pause.load(Ordering::Relaxed)
    }

    pub fn clear_pause(&self) {
        self.inner.pause.store(false, Ordering::Relaxed);
    }
}
```

Modify `src/main.rs`: `mod index;` 추가.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test index::tests`
Expected: 5개 테스트 PASS

- [ ] **Step 5: Commit**

```bash
git add src/index.rs src/main.rs
git commit -m "feat: line offset index with shared indexing status"
```

---

## Task 8: 개행 스캔 함수 (indexer.rs)

**Files:**
- Create: `src/indexer.rs`
- Modify: `src/main.rs` (add `mod indexer;`)

**Interfaces:**
- Consumes: `Encoding` (parse), `LineIndex` (index)
- Produces:
  - `pub fn newline_pattern(enc: Encoding) -> &'static [u8]` — 개행 바이트 패턴
  - `pub fn scan_offsets(bytes: &[u8], start: u64, enc: Encoding) -> Vec<u64>` — bytes 내 각 줄 시작 offset들(start 기준 절대 offset). 첫 줄 시작 포함.

- [ ] **Step 1: 실패하는 테스트 작성**

`src/indexer.rs`:
```rust
use crate::parse::Encoding;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_utf8_lf() {
        // "ab\ncd\nef" → 줄 시작 offset: 0, 3, 6
        let offs = scan_offsets(b"ab\ncd\nef", 0, Encoding::Utf8);
        assert_eq!(offs, vec![0, 3, 6]);
    }

    #[test]
    fn scan_with_start_offset() {
        // 파일 중간(start=100)부터 스캔하면 절대 offset으로 환산
        let offs = scan_offsets(b"ab\ncd", 100, Encoding::Utf8);
        assert_eq!(offs, vec![100, 103]);
    }

    #[test]
    fn scan_crlf_still_lf_boundary() {
        // "ab\r\ncd" → 줄 시작 0, 4 (\n 다음)
        let offs = scan_offsets(b"ab\r\ncd", 0, Encoding::Utf8);
        assert_eq!(offs, vec![0, 4]);
    }

    #[test]
    fn scan_utf16le_lf() {
        // "a\nb" in UTF-16LE = 61 00 0A 00 62 00 ; 개행 0A 00 위치 후 줄 시작
        let bytes = [0x61, 0x00, 0x0A, 0x00, 0x62, 0x00];
        let offs = scan_offsets(&bytes, 0, Encoding::Utf16Le);
        assert_eq!(offs, vec![0, 4]);
    }
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test indexer::tests::scan_utf8_lf`
Expected: FAIL — `scan_offsets` not found

- [ ] **Step 3: 최소 구현 작성**

`src/indexer.rs`에 추가(위 `use` 아래):
```rust
/// 인코딩별 개행 바이트 패턴.
pub fn newline_pattern(enc: Encoding) -> &'static [u8] {
    match enc {
        Encoding::Utf8 | Encoding::Cp949 => &[0x0A],
        Encoding::Utf16Le => &[0x0A, 0x00],
        Encoding::Utf16Be => &[0x00, 0x0A],
    }
}

/// bytes 구간에서 각 줄의 시작 offset(절대값 = start + 로컬)을 반환.
/// 첫 줄 시작(start)을 항상 포함. 개행 바로 다음 위치가 새 줄 시작.
pub fn scan_offsets(bytes: &[u8], start: u64, enc: Encoding) -> Vec<u64> {
    let pat = newline_pattern(enc);
    let mut result = Vec::new();
    if bytes.is_empty() {
        return result;
    }
    result.push(start); // 첫 줄 시작
    let step = pat.len();
    let mut i = 0;
    while i + step <= bytes.len() {
        if &bytes[i..i + step] == pat {
            let next = i + step;
            if next < bytes.len() {
                result.push(start + next as u64);
            }
            i += step;
        } else {
            i += 1;
        }
    }
    result
}
```

Modify `src/main.rs`: `mod indexer;` 추가.

> 주: UTF-16의 개행 패턴을 1바이트씩 슬라이딩하면 정렬 문제가 생길 수 있으나, 실전 파일은 문자 경계가 짝수 바이트라 위 방식으로 충분. 필요 시 step 단위 정렬 스캔으로 개선.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test indexer::tests`
Expected: 4개 테스트 PASS

- [ ] **Step 5: Commit**

```bash
git add src/indexer.rs src/main.rs
git commit -m "feat: newline offset scanning per encoding"
```

---

## Task 9: 백그라운드 인덱싱 워커 (indexer.rs)

**Files:**
- Modify: `src/indexer.rs`

**Interfaces:**
- Consumes: `scan_offsets`, `Encoding`, `LineIndex`, `Source`
- Produces:
  - `pub fn spawn_indexer(source: Arc<Source>, index: LineIndex, enc: Encoding, ctx: egui::Context) -> std::thread::JoinHandle<()>`
  - 워커는 청크(예: 8MB) 단위로 `source`를 스캔해 `index.push_offset`, 진행률 갱신, `pause_requested`면 `Phase::Paused`로 멈춤, 끝까지 가면 `Phase::Complete`. 매 청크 후 `ctx.request_repaint()`.

- [ ] **Step 1: 실패하는 테스트 작성**

`src/indexer.rs` tests 모듈에 추가:
```rust
    use crate::index::{LineIndex, Phase};
    use crate::source;
    use std::io::Write;
    use std::sync::Arc;

    fn temp_file(content: &[u8]) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tv_idx_{}.txt", content.len()));
        std::fs::File::create(&p).unwrap().write_all(content).unwrap();
        p
    }

    #[test]
    fn indexer_indexes_all_lines() {
        let p = temp_file(b"a\nb\nc\nd\n");
        let src = Arc::new(source::open(&p).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();
        let handle = spawn_indexer(src, idx.clone(), Encoding::Utf8, ctx);
        handle.join().unwrap();
        assert_eq!(idx.status().phase, Phase::Complete);
        // "a\nb\nc\nd\n" → 줄 시작: 0,2,4,6 (마지막 개행 뒤 빈 줄은 포함 안 함)
        assert_eq!(idx.line_count(), 4);
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test indexer::tests::indexer_indexes_all_lines`
Expected: FAIL — `spawn_indexer` not found

- [ ] **Step 3: 최소 구현 작성**

`src/indexer.rs`에 추가(상단 `use` 보강):
```rust
use crate::index::{LineIndex, Phase};
use crate::source::Source;
use std::sync::Arc;

const CHUNK: usize = 8 * 1024 * 1024; // 8MB

/// 백그라운드 스레드에서 파일 끝까지 개행 offset을 점진적으로 인덱싱한다.
/// pause_requested가 서면 Paused로 멈추고 스레드 종료(재개는 새 spawn).
pub fn spawn_indexer(
    source: Arc<Source>,
    index: LineIndex,
    enc: Encoding,
    ctx: egui::Context,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        index.set_phase(Phase::Indexing);
        let bytes = source.as_bytes();
        let total = bytes.len();
        // 이미 인덱싱된 만큼 이어서 시작(재개 지원): 마지막 offset + 그 줄 길이는 알 수 없으니
        // 재개는 "다음 스캔 시작 바이트"를 bytes_done 기준으로 삼는다.
        let mut pos = index.status().bytes_done as usize;
        let step = newline_pattern(enc).len();

        // 첫 줄 시작(0)은 인덱스가 비어있을 때만 추가.
        if index.line_count() == 0 && total > 0 {
            index.push_offset(0);
        }

        while pos < total {
            if index.pause_requested() {
                index.clear_pause();
                index.set_phase(Phase::Paused);
                return;
            }
            let end = (pos + CHUNK).min(total);
            // 청크 경계에서 개행 패턴이 잘리지 않도록 step-1 겹침 스캔
            let scan_end = end;
            let slice = &bytes[pos..scan_end];
            let pat = newline_pattern(enc);
            let mut i = 0;
            while i + step <= slice.len() {
                if &slice[i..i + step] == pat {
                    let next_abs = pos + i + step;
                    if next_abs < total {
                        index.push_offset(next_abs as u64);
                    }
                    i += step;
                } else {
                    i += 1;
                }
            }
            pos = end;
            index.set_bytes_done(pos as u64);
            ctx.request_repaint();
        }
        index.set_bytes_done(total as u64);
        index.set_phase(Phase::Complete);
        ctx.request_repaint();
    })
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test indexer::tests::indexer_indexes_all_lines`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/indexer.rs
git commit -m "feat: background indexing worker with pause support"
```

---

## Task 10: 문서 상태 조립 — 파일 열기 오케스트레이션 (app.rs 파트 1)

**Files:**
- Create: `src/app.rs`
- Modify: `src/main.rs` (PlaceholderApp → 실제 App 사용)

**Interfaces:**
- Consumes: `source::open`, `parse::{detect_encoding, detect_delimiter, detect_header, decode_line, split_fields, Encoding}`, `index::LineIndex`, `indexer::spawn_indexer`
- Produces:
  - `pub struct Document { source: Arc<Source>, index: LineIndex, enc: Encoding, delim: u8, has_header: bool, indexer: Option<JoinHandle<()>> }`
  - `pub struct App { doc: Option<Document>, error: Option<String> }`
  - `impl App { pub fn open_path(&mut self, path: &Path, ctx: &egui::Context) }` — 파일 열고 1단계 프라이밍(앞부분 스캔) + 감지 + 워커 spawn.

- [ ] **Step 1: 실패하는 테스트 작성**

`src/app.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(content: &[u8]) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tv_app_{}.csv", content.len()));
        std::fs::File::create(&p).unwrap().write_all(content).unwrap();
        p
    }

    #[test]
    fn open_detects_and_primes() {
        let p = temp(b"name,age\nAlice,30\nBob,25\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_ref().unwrap();
        assert_eq!(doc.enc, crate::parse::Encoding::Utf8);
        assert_eq!(doc.delim, b',');
        assert!(doc.has_header);
        assert!(app.error.is_none());
    }

    #[test]
    fn open_missing_sets_error() {
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(std::path::Path::new("nope_xyz.csv"), &ctx);
        assert!(app.doc.is_none());
        assert!(app.error.is_some());
    }
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test app::tests::open_detects_and_primes`
Expected: FAIL — `App` not found

- [ ] **Step 3: 최소 구현 작성**

`src/app.rs` 맨 위:
```rust
use crate::index::LineIndex;
use crate::indexer;
use crate::parse::{self, Encoding};
use crate::source::{self, Source};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct Document {
    pub source: Arc<Source>,
    pub index: LineIndex,
    pub enc: Encoding,
    pub delim: u8,
    pub has_header: bool,
    pub indexer: Option<JoinHandle<()>>,
    pub path_label: String,
}

#[derive(Default)]
pub struct App {
    pub doc: Option<Document>,
    pub error: Option<String>,
}

/// 프라이밍 시 감지에 쓸 앞부분 바이트 크기.
const PRIME_BYTES: usize = 64 * 1024;

impl App {
    /// 파일을 열고 앞부분으로 인코딩/구분자/헤더를 감지한 뒤 백그라운드 인덱싱을 시작한다.
    pub fn open_path(&mut self, path: &Path, ctx: &egui::Context) {
        self.error = None;
        let src = match source::open(path) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.error = Some(format!("파일 열기 실패: {e}"));
                self.doc = None;
                return;
            }
        };

        let head = {
            let n = (src.len() as usize).min(PRIME_BYTES);
            src.slice(0, n as u64)
        };
        let enc = parse::detect_encoding(head);

        // 앞부분을 줄 단위로 나눠 구분자/헤더 감지에 사용
        let head_text = parse::decode_line(head, enc);
        let head_lines: Vec<&str> = head_text.lines().take(50).collect();
        let delim = parse::detect_delimiter(path, &head_lines);
        let sample_rows: Vec<Vec<String>> = head_lines
            .iter()
            .take(20)
            .map(|l| parse::split_fields(l, delim))
            .collect();
        let has_header = parse::detect_header(&sample_rows);

        let index = LineIndex::new(src.len());
        let handle = indexer::spawn_indexer(src.clone(), index.clone(), enc, ctx.clone());

        self.doc = Some(Document {
            source: src,
            index,
            enc,
            delim,
            has_header,
            indexer: Some(handle),
            path_label: path.display().to_string(),
        });
    }
}
```

Modify `src/main.rs`:
```rust
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
```

이 시점에서 `App`은 아직 `eframe::App`을 구현하지 않아 컴파일이 안 되므로, Task 11에서 `update`를 추가하기 전까지 임시로 `main.rs`에 최소 구현을 넣지 말고 **Task 11과 함께 빌드**한다. 단위테스트는 `update` 없이도 통과하므로 Step 4는 테스트만 실행.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test app::tests`
Expected: 2개 테스트 PASS (`eframe::App` 미구현 경고/에러가 나면 Task 11의 `impl eframe::App`을 먼저 최소 형태로 추가 후 재실행)

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: document open orchestration with priming and detection"
```

---

## Task 11: egui UI — 툴바 · 테이블 · 상태바 (app.rs 파트 2)

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Document`, `LineIndex`, `parse::{decode_line, split_fields}`, `index::Phase`, egui_extras
- Produces: `impl eframe::App for App { fn update(...) }` — 전체 UI 렌더.

- [ ] **Step 1: update 구현 작성 (UI는 수동 검증)**

`src/app.rs`에 추가:
```rust
use crate::index::Phase;
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 20.0;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 상단 툴바
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("파일 열기").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.open_path(&path, ctx);
                    }
                }
                if let Some(doc) = &mut self.doc {
                    ui.separator();
                    // 구분자 드롭다운
                    let delim_label = match doc.delim {
                        b',' => "콤마 ,",
                        b'\t' => "탭",
                        b'|' => "파이프 |",
                        b';' => "세미콜론 ;",
                        _ => "기타",
                    };
                    egui::ComboBox::from_label("구분자")
                        .selected_text(delim_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.delim, b',', "콤마 ,");
                            ui.selectable_value(&mut doc.delim, b'\t', "탭");
                            ui.selectable_value(&mut doc.delim, b'|', "파이프 |");
                            ui.selectable_value(&mut doc.delim, b';', "세미콜론 ;");
                        });
                    // 인코딩 드롭다운
                    let enc_label = format!("{:?}", doc.enc);
                    egui::ComboBox::from_label("인코딩")
                        .selected_text(enc_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf8, "UTF-8");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Cp949, "CP949");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Le, "UTF-16LE");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Be, "UTF-16BE");
                        });
                    ui.checkbox(&mut doc.has_header, "헤더");
                    ui.separator();
                    ui.label(&doc.path_label);
                }
            });
        });

        // 하단 상태바
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::RED, err);
                } else if let Some(doc) = &self.doc {
                    let st = doc.index.status();
                    let done_gb = st.bytes_done as f64 / 1e9;
                    let total_gb = st.total_bytes as f64 / 1e9;
                    let pct = if st.total_bytes > 0 {
                        st.bytes_done * 100 / st.total_bytes
                    } else {
                        100
                    };
                    match st.phase {
                        Phase::Priming | Phase::Indexing => {
                            ui.label(format!(
                                "인덱싱 중... {done_gb:.2} / {total_gb:.2} GB ({pct}%)"
                            ));
                            if ui.button("중단").clicked() {
                                doc.index.request_pause();
                            }
                        }
                        Phase::Paused => {
                            ui.label(format!(
                                "일시정지 — {done_gb:.2} / {total_gb:.2} GB ({pct}%)"
                            ));
                            if ui.button("이어서 읽기").clicked() {
                                // 새 워커 spawn(재개). 기존 핸들은 이미 종료됨.
                                let handle = crate::indexer::spawn_indexer(
                                    doc.source.clone(),
                                    doc.index.clone(),
                                    doc.enc,
                                    ctx.clone(),
                                );
                                doc.indexer = Some(handle);
                            }
                        }
                        Phase::Complete => {
                            ui.label(format!("완료 — {} 행", doc.index.line_count()));
                        }
                    }
                } else {
                    ui.label("파일을 여세요");
                }
            });
        });

        // 본문 테이블
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(doc) = &self.doc else { return };

            // 헤더 행 데이터(있으면 첫 줄)와 데이터 시작 행 결정
            let total_lines = doc.index.line_count();
            let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
                doc.index.line_range(0).map(|(s, e)| {
                    let text = crate::parse::decode_line(doc.source.slice(s, e), doc.enc);
                    crate::parse::split_fields(text.trim_end_matches(['\r', '\n']), doc.delim)
                })
            } else {
                None
            };

            let data_start = if doc.has_header { 1 } else { 0 };
            let data_rows = total_lines.saturating_sub(data_start);
            let col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(1).max(1);

            let mut table = TableBuilder::new(ui)
                .striped(true)
                .column(Column::auto().at_least(48.0)) // 라인번호 #
                .columns(Column::auto().at_least(60.0).resizable(true), col_count);

            table
                .header(ROW_HEIGHT, |mut header| {
                    header.col(|ui| {
                        ui.strong("#");
                    });
                    for c in 0..col_count {
                        header.col(|ui| {
                            if let Some(h) = &header_fields {
                                ui.strong(format!("{} {}", c + 1, h.get(c).cloned().unwrap_or_default()));
                            } else {
                                ui.strong(format!("{}", c + 1));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, data_rows, |mut row| {
                        let logical = row.index() + data_start;
                        let line_no = row.index() + 1;
                        // 라인번호 컬럼
                        row.col(|ui| {
                            ui.label(format!("{line_no}"));
                        });
                        // 데이터 컬럼들 — 이 행만 offset으로 조회·디코딩·파싱
                        let fields = doc
                            .index
                            .line_range(logical)
                            .map(|(s, e)| {
                                let text = crate::parse::decode_line(doc.source.slice(s, e), doc.enc);
                                crate::parse::split_fields(
                                    text.trim_end_matches(['\r', '\n']),
                                    doc.delim,
                                )
                            })
                            .unwrap_or_default();
                        for c in 0..col_count {
                            row.col(|ui| {
                                ui.label(fields.get(c).cloned().unwrap_or_default());
                            });
                        }
                    });
                });
        });
    }
}
```

- [ ] **Step 2: 빌드 확인**

Run: `cargo build`
Expected: 성공. (경고는 허용, 에러 없음)

- [ ] **Step 3: 전체 테스트 확인**

Run: `cargo test`
Expected: 모든 단위 테스트 PASS

- [ ] **Step 4: 수동 스모크 테스트**

작은 CSV로 실행해 눈으로 확인:
```bash
printf "name,age,city\nAlice,30,Seoul\nBob,25,Busan\n" > sample.csv
cargo run --release
```
확인 항목: 파일 열기로 sample.csv 선택 → 헤더행(1 name, 2 age, 3 city) + 라인번호 + 데이터 표시, 상태바 "완료 — 2 행"(헤더 제외 데이터), 헤더 체크박스 토글 시 첫 줄이 데이터로 내려옴, 구분자/인코딩 드롭다운 동작.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs
git commit -m "feat: egui table UI with virtual scroll, toolbar, status bar"
```

---

## Task 12: 대용량 스모크 테스트 + 릴리스 빌드 확인

**Files:**
- Create: `scripts/make_big_csv.ps1` (합성 대용량 파일 생성기)

**Interfaces:**
- Consumes: 전체 앱
- Produces: 릴리스 .exe 동작 확인

- [ ] **Step 1: 합성 파일 생성 스크립트 작성**

`scripts/make_big_csv.ps1`:
```powershell
# 약 1GB CSV 생성 (행 수 조절로 크기 변경)
$path = "big_test.csv"
$rows = 20000000
$sw = [System.IO.StreamWriter]::new($path)
$sw.WriteLine("id,name,value,city")
for ($i = 1; $i -le $rows; $i++) {
    $sw.WriteLine("$i,name$i,$($i * 3),city$($i % 100)")
}
$sw.Close()
Write-Host "created $path"
```

- [ ] **Step 2: 릴리스 빌드**

Run: `cargo build --release`
Expected: `target/release/textviewer.exe` 생성

- [ ] **Step 3: 대용량 파일 생성 후 열기 (수동)**

PowerShell:
```
powershell -ExecutionPolicy Bypass -File scripts/make_big_csv.ps1
.\target\release\textviewer.exe
```
확인 항목:
- 파일 열기 → big_test.csv 선택 시 **즉시**(0.5초 이내) 앞부분 테이블 표시
- 하단 상태바에 인덱싱 진행률 텍스트가 올라감
- 인덱싱 중 [중단] → [이어서 읽기] 토글 동작, 중단 시 읽은 데까지만 스크롤
- 인덱싱 완료 후 끝까지 스크롤 부드러움(체감 끊김 없음)
- 헤더/구분자/인코딩 드롭다운 동작

- [ ] **Step 4: 결과 기록**

첫 표시 시간, 인덱싱 완료 시간, 스크롤 체감을 `.readme/`에 간단히 기록(선택).

- [ ] **Step 5: Commit**

```bash
git add scripts/make_big_csv.ps1
git commit -m "chore: large-file smoke test script"
```

---

## Self-Review 체크 결과

**Spec coverage:**
- 즉시 표시(2단계) → Task 10(프라이밍) + Task 9(백그라운드) + Task 11(가상 스크롤 렌더) ✓
- 구분자 감지/변경 → Task 4 + Task 11 드롭다운 ✓
- 헤더 감지/토글 → Task 5 + Task 11 체크박스 ✓
- 인코딩 감지/변경(UTF-8/CP949/UTF-16) → Task 2,3 + Task 11 드롭다운 ✓
- 라인번호 + 헤더 위 컬럼번호 → Task 11 ✓
- 중단/이어서 토글 → Task 7(pause 플래그) + Task 9(워커) + Task 11(버튼) ✓
- 인덱싱 중 읽은 데까지만 스크롤 → Task 11에서 `data_rows = line_count - header` 사용(자라나는 값) ✓
- 단일 .exe → Task 1 프로필 + Task 12 릴리스 빌드 ✓
- mmap 낮은 메모리 → Task 6 ✓
- 크래시 금지(lossy 디코딩, 필드 부족 시 빈 셀) → Task 3, Task 11 ✓

**Placeholder scan:** 모든 스텝에 실제 코드/명령 포함. TODO/TBD 없음. ✓

**Type consistency:**
- `Encoding` enum: Task 2 정의, Task 3/8/10/11에서 동일 사용 ✓
- `LineIndex` 메서드(`push_offset`, `line_count`, `line_range`, `status`, `request_pause`, `pause_requested`, `clear_pause`, `set_phase`, `set_bytes_done`): Task 7 정의, Task 9/11 사용 일치 ✓
- `Source`(`open`, `len`, `slice`, `as_bytes`): Task 6 정의, Task 9/10/11 사용 일치 ✓
- `spawn_indexer` 시그니처: Task 9 정의, Task 10/11 호출 일치 ✓
- `split_fields`/`decode_line`/`detect_*`: Task 2~5 정의, Task 10/11 사용 일치 ✓

**알려진 리스크(구현 중 조정 가능):**
- Task 6 빈 파일 mmap 처리 — `map_anon` 조합이 안 되면 `empty` 플래그 방식으로 대체(주석에 명시).
- Task 8/9 UTF-16 개행 스캔의 바이트 정렬 — 실전 파일은 짝수 경계라 문제없으나, 필요 시 step 정렬 스캔으로 개선.
- egui/eframe 0.28 API — `body.rows` 콜백에서 `row.index()`/`row.col()` 사용. 버전 상이 시 시그니처 확인.
