# 편집 모드 + 저장/인코딩 변환 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 뷰 전용(mmap) 대용량 뷰어에 편집 기능을 추가한다 — 텍스트 모드는 자유 텍스트 편집, 세퍼레이터 모드는 셀 단위 편집. 드래그 선택·통삭제·통붙여넣기·우클릭 메뉴를 지원하고, 저장/다른 이름으로 저장 시 인코딩(UTF-8/CP949/UTF-16LE/BE)을 변환한다.

**Architecture:** 편집 모드 진입 시 파일 전체를 `Vec<String>`(줄 배열, `EditBuffer.lines`)로 RAM에 로드한다. 뷰 경로(mmap+LineIndex)는 유지하되, `Document.edit: Option<EditBuffer>`가 Some이면 렌더·정렬·저장이 모두 `EditBuffer`를 진실의 원천으로 삼는다. 편집 모드에서 정렬은 permutation이 아니라 `lines`를 실제로 재배치하는 1회 편집이다. 저장은 `lines`를 대상 인코딩으로 재인코딩하며 임시 파일에 스트리밍 후 원자적 rename 한다.

**Tech Stack:** Rust, egui/eframe 0.28, egui_extras(TableBuilder), encoding_rs, memchr, rfd(파일 다이얼로그), rayon(인메모리 정렬).

## Global Constraints

- 순수 로직(edit.rs/save.rs)은 GUI 없이 단위 테스트 가능해야 한다. egui 타입 의존 금지.
- 줄 문자열은 항상 개행 문자를 포함하지 않는다(`lines[i]`에 `\n`/`\r` 없음). 개행은 `newline` 필드로 별도 관리, 저장 시에만 부여.
- 셀 값 재조립 시 CSV 인용 규칙: 값에 `delim` / `"` / `\n` / `\r` 중 하나라도 있으면 전체를 `"`로 감싸고 내부 `"`는 `""`로 이스케이프.
- 저장은 임시 파일(`<대상>.tmp`)에 다 쓴 뒤 `std::fs::rename`으로 교체. 실패 시 임시 파일 삭제, 원본 보존.
- 인메모리 정렬은 기존 sort.rs의 키 인코딩(text_key/number_key/col_key 정신)을 재사용하되, mmap/offset 대신 `&[String]`에서 직접 필드를 뽑는다.
- 편집 모드에서 정렬은 `lines`를 실제 재배치한다(헤더 행 lines[0]은 has_header면 제외·맨 앞 고정). permutation 뷰 매핑은 뷰 전용 모드에서만.
- 커밋 메시지 말미: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- Windows 환경, 로컬 작업만(origin push는 명시 지시 없이는 안 함).

---

### Task 1: EditBuffer 로드 (파일 → 줄 배열)

**Files:**
- Create: `src/edit.rs`
- Modify: `src/main.rs` (또는 `src/lib.rs` — `mod edit;` 추가)
- Test: `src/edit.rs` (하단 `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::source::Source`(`as_bytes() -> &[u8]`), `crate::parse::{Encoding, decode_line}`.
- Produces:
  - `pub struct EditBuffer { pub lines: Vec<String>, pub dirty: bool, pub newline: Newline }`
  - `pub enum Newline { Lf, CrLf }`
  - `pub fn load_edit_buffer(source: &crate::source::Source, enc: crate::parse::Encoding) -> EditBuffer`

- [ ] **Step 1: main.rs에 모듈 선언 추가**

`src/main.rs`(또는 lib.rs) 상단 `mod` 목록에 한 줄 추가:

```rust
mod edit;
```

기존 `mod sort;` 등이 있는 곳 바로 아래에 둔다. 위치를 모르면 `grep -n "^mod " src/main.rs`로 확인.

- [ ] **Step 2: 실패하는 테스트 작성**

`src/edit.rs`를 새로 만들고 아래 내용을 넣는다(본문 함수는 아직 비어 컴파일만 되게):

```rust
use crate::parse::{decode_line, Encoding};
use crate::source::Source;

/// 원본 파일의 개행 스타일. 저장 시 재현한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    CrLf,
}

/// 편집 모드의 인메모리 문서. 파일 전체를 줄 단위로 보관한다.
/// lines[i]에는 개행 문자가 포함되지 않는다.
pub struct EditBuffer {
    pub lines: Vec<String>,
    pub dirty: bool,
    pub newline: Newline,
}

/// mmap 바이트를 개행(`\n`)으로 분할하고 각 줄을 `enc`로 디코딩해 EditBuffer를 만든다.
/// - `\r\n`이 하나라도 있으면 newline=CrLf, 아니면 Lf.
/// - 각 줄에서 뒤쪽 `\r`/`\n`을 제거한다.
/// - 마지막 바이트가 개행이면 그 뒤에 빈 줄을 추가하지 않는다(파일 끝 개행은 종결자).
/// - 빈 파일이면 `lines = [""]`(빈 한 줄).
pub fn load_edit_buffer(source: &Source, enc: Encoding) -> EditBuffer {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_lf_basic() {
        // "a\nb\nc\n" → ["a","b","c"], newline=Lf
        let src = crate::source::Source::from_bytes_for_test(b"a\nb\nc\n");
        let buf = load_edit_buffer(&src, Encoding::Utf8);
        assert_eq!(buf.lines, vec!["a", "b", "c"]);
        assert_eq!(buf.newline, Newline::Lf);
        assert!(!buf.dirty);
    }

    #[test]
    fn load_crlf_detected() {
        let src = crate::source::Source::from_bytes_for_test(b"a\r\nb\r\n");
        let buf = load_edit_buffer(&src, Encoding::Utf8);
        assert_eq!(buf.lines, vec!["a", "b"]);
        assert_eq!(buf.newline, Newline::CrLf);
    }

    #[test]
    fn load_no_trailing_newline() {
        // 마지막 줄에 개행이 없으면 그 줄도 포함.
        let src = crate::source::Source::from_bytes_for_test(b"a\nb");
        let buf = load_edit_buffer(&src, Encoding::Utf8);
        assert_eq!(buf.lines, vec!["a", "b"]);
    }

    #[test]
    fn load_empty_file_is_one_empty_line() {
        let src = crate::source::Source::from_bytes_for_test(b"");
        let buf = load_edit_buffer(&src, Encoding::Utf8);
        assert_eq!(buf.lines, vec![""]);
        assert_eq!(buf.newline, Newline::Lf);
    }

    #[test]
    fn load_cp949_decodes() {
        // "가나" in CP949 = B0 A1 B3 AA, 한 줄.
        let src = crate::source::Source::from_bytes_for_test(&[0xB0, 0xA1, 0xB3, 0xAA, b'\n']);
        let buf = load_edit_buffer(&src, Encoding::Cp949);
        assert_eq!(buf.lines, vec!["가나"]);
    }
}
```

- [ ] **Step 3: Source에 테스트용 생성자 추가**

`src/source.rs`의 `impl Source` 블록에 아래를 추가한다(테스트에서 파일 없이 바이트로 Source를 만들기 위함). mmap 대신 힙 바이트를 들고 있도록 내부를 확장:

`src/source.rs` 상단 struct 정의를 다음으로 교체:

```rust
pub struct Source {
    mmap: Option<Mmap>,
    /// 테스트용: 파일 없이 메모리 바이트로 만든 소스. mmap과 배타적.
    owned: Option<Vec<u8>>,
}
```

`open()`의 반환 두 곳(`Source { mmap: None }`, `Source { mmap: Some(mmap) }`)에 `owned: None`을 추가한다. 그리고 `len`/`slice`/`as_bytes`가 owned도 처리하도록 수정:

```rust
impl Source {
    /// 테스트 전용: 메모리 바이트로 Source를 만든다.
    #[cfg(test)]
    pub fn from_bytes_for_test(bytes: &[u8]) -> Source {
        Source { mmap: None, owned: Some(bytes.to_vec()) }
    }

    fn bytes(&self) -> &[u8] {
        if let Some(m) = &self.mmap {
            &m[..]
        } else if let Some(v) = &self.owned {
            &v[..]
        } else {
            &[]
        }
    }

    pub fn len(&self) -> u64 {
        self.bytes().len() as u64
    }

    pub fn slice(&self, start: u64, end: u64) -> &[u8] {
        let b = self.bytes();
        let len = b.len() as u64;
        let s = start.min(len) as usize;
        let e = end.min(len).max(start.min(len)) as usize;
        &b[s..e]
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.bytes()
    }
}
```

기존 `len`/`slice`/`as_bytes` 본문은 위 버전으로 대체(중복 정의 남기지 말 것).

- [ ] **Step 4: 테스트 실패 확인**

Run: `cargo test edit::tests`
Expected: FAIL — `load_edit_buffer`가 `todo!()`라 패닉.

- [ ] **Step 5: load_edit_buffer 구현**

`src/edit.rs`의 `load_edit_buffer`를 구현:

```rust
pub fn load_edit_buffer(source: &Source, enc: Encoding) -> EditBuffer {
    let bytes = source.as_bytes();
    if bytes.is_empty() {
        return EditBuffer { lines: vec![String::new()], dirty: false, newline: Newline::Lf };
    }
    let mut newline = Newline::Lf;
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(pos) = memchr::memchr(b'\n', &bytes[i..]) {
            let nl = i + pos; // '\n' 위치
            let mut end = nl;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
                newline = Newline::CrLf;
            }
            lines.push(decode_line(&bytes[start..end], enc));
            start = nl + 1;
            i = nl + 1;
        } else {
            break;
        }
    }
    // 마지막 개행 뒤에 내용이 남아 있으면(파일 끝 개행 없음) 그 줄도 추가.
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
            newline = Newline::CrLf;
        }
        lines.push(decode_line(&bytes[start..end], enc));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    EditBuffer { lines, dirty: false, newline }
}
```

- [ ] **Step 6: 테스트 통과 확인**

Run: `cargo test edit::tests`
Expected: PASS (5 tests).

- [ ] **Step 7: 커밋**

```bash
git add src/edit.rs src/source.rs src/main.rs
git commit -m "feat: EditBuffer 로드 - 파일을 줄 배열로 디코딩"
```

---

### Task 2: 텍스트 편집 연산 (커서/선택/삽입/삭제/줄 분할·병합)

**Files:**
- Modify: `src/edit.rs`
- Test: `src/edit.rs` (tests 모듈)

**Interfaces:**
- Consumes: `EditBuffer.lines`(Task 1).
- Produces:
  - `pub struct TextPos { pub line: usize, pub col: usize }` (col = 문자 인덱스, char 단위)
  - `pub fn insert_char(lines, pos, ch) -> TextPos` (새 커서 위치 반환)
  - `pub fn insert_str(lines, pos, s) -> TextPos` (개행 포함 문자열 삽입, 줄 분할)
  - `pub fn split_line(lines, pos) -> TextPos` (Enter — pos에서 줄을 둘로)
  - `pub fn delete_range(lines, a, b) -> TextPos` (정규화된 [a,b) 삭제, 병합 포함)
  - `pub fn backspace(lines, pos) -> TextPos` (한 문자/줄 병합 삭제)
  - `pub fn normalize(a, b) -> (TextPos, TextPos)` (a<=b 되도록)

**Interfaces 상세:** col은 char 인덱스(바이트 아님). 줄 내 편집은 char 경계에서만.

- [ ] **Step 1: 실패하는 테스트 작성**

`src/edit.rs` tests 모듈에 추가:

```rust
    fn v(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn insert_char_mid_line() {
        let mut lines = v(&["abc"]);
        let p = insert_char(&mut lines, TextPos { line: 0, col: 1 }, 'X');
        assert_eq!(lines, v(&["aXbc"]));
        assert_eq!(p, TextPos { line: 0, col: 2 });
    }

    #[test]
    fn split_line_enter() {
        let mut lines = v(&["abcd"]);
        let p = split_line(&mut lines, TextPos { line: 0, col: 2 });
        assert_eq!(lines, v(&["ab", "cd"]));
        assert_eq!(p, TextPos { line: 1, col: 0 });
    }

    #[test]
    fn backspace_mid_line() {
        let mut lines = v(&["abc"]);
        let p = backspace(&mut lines, TextPos { line: 0, col: 2 });
        assert_eq!(lines, v(&["ac"]));
        assert_eq!(p, TextPos { line: 0, col: 1 });
    }

    #[test]
    fn backspace_at_line_start_merges() {
        // 줄 맨 앞 Backspace → 앞 줄과 병합, 커서는 병합점.
        let mut lines = v(&["ab", "cd"]);
        let p = backspace(&mut lines, TextPos { line: 1, col: 0 });
        assert_eq!(lines, v(&["abcd"]));
        assert_eq!(p, TextPos { line: 0, col: 2 });
    }

    #[test]
    fn backspace_at_origin_noop() {
        let mut lines = v(&["ab"]);
        let p = backspace(&mut lines, TextPos { line: 0, col: 0 });
        assert_eq!(lines, v(&["ab"]));
        assert_eq!(p, TextPos { line: 0, col: 0 });
    }

    #[test]
    fn delete_range_within_line() {
        let mut lines = v(&["abcdef"]);
        let p = delete_range(
            &mut lines,
            TextPos { line: 0, col: 1 },
            TextPos { line: 0, col: 4 },
        );
        assert_eq!(lines, v(&["aef"]));
        assert_eq!(p, TextPos { line: 0, col: 1 });
    }

    #[test]
    fn delete_range_multiline() {
        // 1행 col1 ~ 3행 col2 삭제 → 시작 줄 앞부분 + 끝 줄 뒷부분 병합.
        let mut lines = v(&["abc", "XXX", "defg"]);
        let p = delete_range(
            &mut lines,
            TextPos { line: 0, col: 1 },
            TextPos { line: 2, col: 2 },
        );
        // "a" + "fg" = "afg"
        assert_eq!(lines, v(&["afg"]));
        assert_eq!(p, TextPos { line: 0, col: 1 });
    }

    #[test]
    fn insert_str_with_newlines() {
        let mut lines = v(&["ad"]);
        let p = insert_str(&mut lines, TextPos { line: 0, col: 1 }, "b\nc");
        // "a" + "b" / "c" + "d" → ["ab","cd"]
        assert_eq!(lines, v(&["ab", "cd"]));
        assert_eq!(p, TextPos { line: 1, col: 1 });
    }

    #[test]
    fn normalize_swaps_when_reversed() {
        let a = TextPos { line: 2, col: 0 };
        let b = TextPos { line: 1, col: 3 };
        assert_eq!(normalize(a, b), (b, a));
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test edit::tests`
Expected: FAIL — 함수 미정의(컴파일 에러).

- [ ] **Step 3: 텍스트 편집 연산 구현**

`src/edit.rs`에 추가(char 단위 처리를 위해 char_indices 사용):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPos {
    pub line: usize,
    pub col: usize, // 문자(char) 인덱스
}

/// 문자열의 char 인덱스 col을 바이트 오프셋으로 변환(끝이면 len).
fn byte_of(s: &str, col: usize) -> usize {
    s.char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// 한 줄의 char 개수.
fn char_len(s: &str) -> usize {
    s.chars().count()
}

pub fn normalize(a: TextPos, b: TextPos) -> (TextPos, TextPos) {
    if (a.line, a.col) <= (b.line, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

pub fn insert_char(lines: &mut Vec<String>, pos: TextPos, ch: char) -> TextPos {
    let b = byte_of(&lines[pos.line], pos.col);
    lines[pos.line].insert(b, ch);
    TextPos { line: pos.line, col: pos.col + 1 }
}

pub fn split_line(lines: &mut Vec<String>, pos: TextPos) -> TextPos {
    let b = byte_of(&lines[pos.line], pos.col);
    let tail = lines[pos.line].split_off(b);
    lines.insert(pos.line + 1, tail);
    TextPos { line: pos.line + 1, col: 0 }
}

pub fn insert_str(lines: &mut Vec<String>, pos: TextPos, s: &str) -> TextPos {
    // s를 개행으로 나눠 삽입. 개행 없으면 단순 삽입.
    let mut parts = s.split('\n');
    let first = parts.next().unwrap_or("");
    let b = byte_of(&lines[pos.line], pos.col);
    // 현재 줄을 삽입 지점에서 자른다.
    let tail = lines[pos.line].split_off(b);
    lines[pos.line].push_str(first);
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        // 개행 없음: tail을 다시 붙이고 커서는 first 끝.
        let col = pos.col + char_len(first);
        lines[pos.line].push_str(&tail);
        return TextPos { line: pos.line, col };
    }
    // 개행 있음: 중간 줄들 삽입, 마지막 줄에 tail 붙임.
    let mut cur = pos.line;
    for (k, seg) in rest.iter().enumerate() {
        cur += 1;
        if k + 1 == rest.len() {
            let mut last = seg.to_string();
            let col = char_len(&last);
            last.push_str(&tail);
            lines.insert(cur, last);
            return TextPos { line: cur, col };
        } else {
            lines.insert(cur, seg.to_string());
        }
    }
    unreachable!()
}

pub fn delete_range(lines: &mut Vec<String>, a: TextPos, b: TextPos) -> TextPos {
    let (a, b) = normalize(a, b);
    if a == b {
        return a;
    }
    if a.line == b.line {
        let sa = byte_of(&lines[a.line], a.col);
        let sb = byte_of(&lines[a.line], b.col);
        lines[a.line].replace_range(sa..sb, "");
        return a;
    }
    // 멀티라인: a.line의 앞부분 + b.line의 뒷부분 병합, 사이 줄 제거.
    let sa = byte_of(&lines[a.line], a.col);
    let head = lines[a.line][..sa].to_string();
    let sb = byte_of(&lines[b.line], b.col);
    let tail = lines[b.line][sb..].to_string();
    let merged = head + &tail;
    lines[a.line] = merged;
    // a.line+1 ..= b.line 제거.
    lines.drain(a.line + 1..=b.line);
    a
}

pub fn backspace(lines: &mut Vec<String>, pos: TextPos) -> TextPos {
    if pos.col > 0 {
        let prev = TextPos { line: pos.line, col: pos.col - 1 };
        return delete_range(lines, prev, pos);
    }
    if pos.line == 0 {
        return pos; // 문서 맨 앞: no-op
    }
    // 줄 맨 앞: 앞 줄과 병합. 병합점 = 앞 줄의 기존 끝.
    let prev_len = char_len(&lines[pos.line - 1]);
    let merge = TextPos { line: pos.line - 1, col: prev_len };
    delete_range(lines, merge, pos)
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test edit::tests`
Expected: PASS (Task 1의 5 + Task 2의 9 = 14 tests).

- [ ] **Step 5: 커밋**

```bash
git add src/edit.rs
git commit -m "feat: 텍스트 편집 연산 - 삽입/삭제/줄 분할·병합/범위 삭제"
```

---

### Task 3: 셀 편집 연산 (필드 교체·재조립, 행 삽입/삭제)

**Files:**
- Modify: `src/edit.rs`, `src/parse.rs`
- Test: `src/edit.rs`, `src/parse.rs`

**Interfaces:**
- Consumes: `crate::parse::{split_fields, field 조립}`.
- Produces:
  - `parse.rs`: `pub fn join_fields(fields: &[String], delim: u8) -> String` (인용 규칙 적용)
  - `edit.rs`: `pub fn set_cell(lines, logical, col, value, delim)` (필드 교체+재조립)
  - `edit.rs`: `pub fn clear_cells(lines, r0, c0, r1, c1, delim)` (사각 영역 빈 값)
  - `edit.rs`: `pub fn insert_row(lines, at, text)` / `pub fn remove_row(lines, at)`

- [ ] **Step 1: parse.rs에 join_fields 실패 테스트**

`src/parse.rs` tests 모듈에 추가:

```rust
    #[test]
    fn join_basic() {
        let f = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(join_fields(&f, b','), "a,b,c");
    }

    #[test]
    fn join_quotes_field_with_delim() {
        let f = vec!["a,b".to_string(), "c".to_string()];
        assert_eq!(join_fields(&f, b','), "\"a,b\",c");
    }

    #[test]
    fn join_escapes_inner_quote() {
        let f = vec!["a\"b".to_string()];
        // 값에 " 있으면 감싸고 내부 "는 "" 로.
        assert_eq!(join_fields(&f, b','), "\"a\"\"b\"");
    }

    #[test]
    fn join_quotes_field_with_newline() {
        let f = vec!["a\nb".to_string()];
        assert_eq!(join_fields(&f, b','), "\"a\nb\"");
    }

    #[test]
    fn join_tab_delim() {
        let f = vec!["x".to_string(), "y".to_string()];
        assert_eq!(join_fields(&f, b'\t'), "x\ty");
    }
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test parse::tests::join`
Expected: FAIL — `join_fields` 미정의.

- [ ] **Step 3: join_fields 구현**

`src/parse.rs`에 추가:

```rust
/// 필드 배열을 구분자로 합쳐 한 줄 문자열을 만든다. CSV 인용 규칙:
/// 값에 delim / `"` / 개행(`\n`/`\r`)이 있으면 전체를 `"`로 감싸고 내부 `"`는 `""`로.
pub fn join_fields(fields: &[String], delim: u8) -> String {
    let delim_ch = delim as char;
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(delim_ch);
        }
        let needs_quote = f.chars().any(|c| {
            c == delim_ch || c == '"' || c == '\n' || c == '\r'
        });
        if needs_quote {
            out.push('"');
            for c in f.chars() {
                if c == '"' {
                    out.push('"');
                }
                out.push(c);
            }
            out.push('"');
        } else {
            out.push_str(f);
        }
    }
    out
}
```

- [ ] **Step 4: join_fields 테스트 통과 확인**

Run: `cargo test parse::tests::join`
Expected: PASS (5 tests).

- [ ] **Step 5: edit.rs 셀/행 연산 실패 테스트**

`src/edit.rs` tests 모듈에 추가:

```rust
    #[test]
    fn set_cell_replaces_field() {
        let mut lines = v(&["a,b,c"]);
        set_cell(&mut lines, 0, 1, "Z", b',');
        assert_eq!(lines, v(&["a,Z,c"]));
    }

    #[test]
    fn set_cell_pads_missing_columns() {
        // col 3을 설정하는데 필드가 2개뿐 → 빈 필드로 패딩.
        let mut lines = v(&["a,b"]);
        set_cell(&mut lines, 0, 3, "Z", b',');
        assert_eq!(lines, v(&["a,b,,Z"]));
    }

    #[test]
    fn set_cell_quotes_when_value_has_delim() {
        let mut lines = v(&["a,b"]);
        set_cell(&mut lines, 0, 0, "x,y", b',');
        assert_eq!(lines, v(&["\"x,y\",b"]));
    }

    #[test]
    fn clear_cells_rectangle() {
        // 2x2 영역(행0~1, 열0~1)을 빈 값으로.
        let mut lines = v(&["a,b,c", "d,e,f", "g,h,i"]);
        clear_cells(&mut lines, 0, 0, 1, 1, b',');
        assert_eq!(lines, v(&[",,c", ",,f", "g,h,i"]));
    }

    #[test]
    fn insert_and_remove_row() {
        let mut lines = v(&["a", "b"]);
        insert_row(&mut lines, 1, String::new());
        assert_eq!(lines, v(&["a", "", "b"]));
        remove_row(&mut lines, 1);
        assert_eq!(lines, v(&["a", "b"]));
    }

    #[test]
    fn remove_last_row_keeps_one_empty() {
        // 마지막 한 줄을 지우면 빈 한 줄은 남긴다(빈 lines 방지).
        let mut lines = v(&["only"]);
        remove_row(&mut lines, 0);
        assert_eq!(lines, v(&[""]));
    }
```

- [ ] **Step 6: 실패 확인**

Run: `cargo test edit::tests`
Expected: FAIL — 함수 미정의.

- [ ] **Step 7: 셀/행 연산 구현**

`src/edit.rs`에 추가(`crate::parse` 사용):

```rust
use crate::parse::{join_fields, split_fields};

/// logical 행의 col번째 필드를 value로 교체하고 줄을 재조립한다.
/// col이 현재 필드 수보다 크면 빈 필드로 패딩한다.
pub fn set_cell(lines: &mut [String], logical: usize, col: usize, value: &str, delim: u8) {
    let mut fields = split_fields(&lines[logical], delim);
    if col >= fields.len() {
        fields.resize(col + 1, String::new());
    }
    fields[col] = value.to_string();
    lines[logical] = join_fields(&fields, delim);
}

/// 사각 영역 [r0..=r1] x [c0..=c1]의 각 셀을 빈 값으로 만든다.
pub fn clear_cells(lines: &mut [String], r0: usize, c0: usize, r1: usize, c1: usize, delim: u8) {
    let (r0, r1) = (r0.min(r1), r0.max(r1));
    let (c0, c1) = (c0.min(c1), c0.max(c1));
    for r in r0..=r1 {
        if r >= lines.len() {
            break;
        }
        let mut fields = split_fields(&lines[r], delim);
        let hi = c1.min(fields.len().saturating_sub(1));
        for c in c0..=hi {
            if c < fields.len() {
                fields[c].clear();
            }
        }
        lines[r] = join_fields(&fields, delim);
    }
}

pub fn insert_row(lines: &mut Vec<String>, at: usize, text: String) {
    let at = at.min(lines.len());
    lines.insert(at, text);
}

pub fn remove_row(lines: &mut Vec<String>, at: usize) {
    if at < lines.len() {
        lines.remove(at);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
}
```

- [ ] **Step 8: 테스트 통과 확인**

Run: `cargo test`
Expected: PASS (edit + parse 전부).

- [ ] **Step 9: 커밋**

```bash
git add src/edit.rs src/parse.rs
git commit -m "feat: 셀/행 편집 연산 - 필드 교체·재조립(join_fields), 행 삽입/삭제"
```

---

### Task 4: 셀 클립보드 (TSV 복사/붙여넣기)

**Files:**
- Modify: `src/edit.rs`
- Test: `src/edit.rs`

**Interfaces:**
- Produces:
  - `pub fn cells_to_tsv(lines, r0, c0, r1, c1, delim) -> String` (선택 영역 → TSV 문자열)
  - `pub fn paste_tsv(lines, r0, c0, tsv, delim)` (TSV를 (r0,c0)부터 그리드로 덮어쓰기, 경계 확장)

- [ ] **Step 1: 실패 테스트**

`src/edit.rs` tests에 추가:

```rust
    #[test]
    fn cells_to_tsv_basic() {
        let lines = v(&["a,b,c", "d,e,f"]);
        // 열 0~1, 행 0~1 → "a\tb\nd\te"
        let s = cells_to_tsv(&lines, 0, 0, 1, 1, b',');
        assert_eq!(s, "a\tb\nd\te");
    }

    #[test]
    fn paste_tsv_overwrites_grid() {
        let mut lines = v(&["a,b,c", "d,e,f"]);
        // (0,1)부터 "X\tY\nZ\tW" 붙여넣기 → 행0 col1,2 = X,Y / 행1 col1,2 = Z,W
        paste_tsv(&mut lines, 0, 1, "X\tY\nZ\tW", b',');
        assert_eq!(lines, v(&["a,X,Y", "d,Z,W"]));
    }

    #[test]
    fn paste_tsv_extends_rows_and_cols() {
        // 파일 경계를 넘는 붙여넣기 → 행/열 확장.
        let mut lines = v(&["a"]);
        paste_tsv(&mut lines, 0, 0, "1\t2\n3\t4", b',');
        assert_eq!(lines, v(&["1,2", "3,4"]));
    }
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test edit::tests`
Expected: FAIL.

- [ ] **Step 3: 구현**

`src/edit.rs`에 추가:

```rust
/// 선택 사각 영역을 TSV(행=\n, 열=\t)로 직렬화한다.
pub fn cells_to_tsv(lines: &[String], r0: usize, c0: usize, r1: usize, c1: usize, delim: u8) -> String {
    let (r0, r1) = (r0.min(r1), r0.max(r1));
    let (c0, c1) = (c0.min(c1), c0.max(c1));
    let mut out = String::new();
    for r in r0..=r1 {
        if r > r0 {
            out.push('\n');
        }
        if r >= lines.len() {
            continue;
        }
        let fields = split_fields(&lines[r], delim);
        for c in c0..=c1 {
            if c > c0 {
                out.push('\t');
            }
            if let Some(f) = fields.get(c) {
                out.push_str(f);
            }
        }
    }
    out
}

/// TSV 클립보드를 (r0,c0)부터 셀 그리드로 덮어쓴다. 경계를 넘으면 행/열 확장.
pub fn paste_tsv(lines: &mut Vec<String>, r0: usize, c0: usize, tsv: &str, delim: u8) {
    for (dr, row) in tsv.split('\n').enumerate() {
        let r = r0 + dr;
        while r >= lines.len() {
            lines.push(String::new());
        }
        let mut fields = split_fields(&lines[r], delim);
        for (dc, cell) in row.split('\t').enumerate() {
            let c = c0 + dc;
            if c >= fields.len() {
                fields.resize(c + 1, String::new());
            }
            fields[c] = cell.to_string();
        }
        lines[r] = join_fields(&fields, delim);
    }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test edit::tests`
Expected: PASS.

- [ ] **Step 5: 커밋**

```bash
git add src/edit.rs
git commit -m "feat: 셀 클립보드 - TSV 복사/붙여넣기(그리드 덮어쓰기·경계 확장)"
```

---

### Task 5: 인메모리 정렬 (편집 모드 lines 재배치)

**Files:**
- Modify: `src/sort.rs`, `src/edit.rs`
- Test: `src/sort.rs`, `src/edit.rs`

**Interfaces:**
- Consumes: `crate::sort::{SortSpec, SortKind, SortDir}` 및 내부 키 함수. 키 함수 `text_key`/`number_key`/`col_key`는 현재 `sort.rs` private이므로 `pub(crate)`로 승격한다.
- Produces:
  - `sort.rs`: `text_key`/`number_key`/`col_key`를 `pub(crate)`로 노출.
  - `sort.rs`: `pub fn sort_lines(lines: &[String], specs: &[SortSpec], delim: u8, data_start: usize) -> Vec<u32>` — 각 줄에서 specs 컬럼 키를 뽑아 정렬, 데이터 행(data_start..)의 순서 permutation(원본 논리 행번호) 반환.
  - `edit.rs`: `pub fn apply_permutation(lines: &mut Vec<String>, order: &[u32], data_start: usize)` — order 순서로 데이터 행 재배치(헤더 유지).

- [ ] **Step 1: sort.rs 키 함수 pub(crate) 승격**

`src/sort.rs`에서 다음 세 함수의 `fn`을 `pub(crate) fn`으로 바꾼다:
- `fn text_key(` → `pub(crate) fn text_key(`
- `fn number_key(` → `pub(crate) fn number_key(`
- `fn col_key(` → `pub(crate) fn col_key(`

(다른 시그니처·본문 변경 없음.)

- [ ] **Step 2: sort_lines 실패 테스트**

`src/sort.rs` tests 모듈에 추가:

```rust
    #[test]
    fn sort_lines_text_ascending() {
        let lines = vec![
            "banana".to_string(),
            "apple".to_string(),
            "cherry".to_string(),
        ];
        let specs = [spec(0, SortKind::Text, SortDir::Asc)];
        let order = sort_lines(&lines, &specs, b',', 0);
        assert_eq!(order, vec![1, 0, 2]);
    }

    #[test]
    fn sort_lines_multi_matches_mmap_path() {
        // 인메모리 정렬이 mmap 경로(extract_and_multi_sort)와 같은 결과.
        let content = b"B,30\nA,20\nB,10\nA,40\n";
        let (src, idx) = open_indexed(content);
        let specs = [
            spec(0, SortKind::Text, SortDir::Asc),
            spec(1, SortKind::Number, SortDir::Asc),
        ];
        let mmap_order = extract_and_multi_sort(&src, &idx, Encoding::Utf8, b',', &specs, 0, None);
        let lines: Vec<String> = "B,30\nA,20\nB,10\nA,40"
            .split('\n')
            .map(|s| s.to_string())
            .collect();
        let mem_order = sort_lines(&lines, &specs, b',', 0);
        assert_eq!(mmap_order, mem_order);
    }

    #[test]
    fn sort_lines_respects_data_start() {
        // data_start=1이면 헤더(0행) 제외, 데이터만 정렬한 논리 행번호 반환.
        let lines = vec![
            "name".to_string(),
            "banana".to_string(),
            "apple".to_string(),
        ];
        let specs = [spec(0, SortKind::Text, SortDir::Asc)];
        let order = sort_lines(&lines, &specs, b',', 1);
        // apple(2), banana(1) → [2,1]
        assert_eq!(order, vec![2, 1]);
    }
```

- [ ] **Step 3: 실패 확인**

Run: `cargo test sort::tests::sort_lines`
Expected: FAIL — `sort_lines` 미정의.

- [ ] **Step 4: sort_lines 구현**

`src/sort.rs`에 추가(rayon 사용, `col_key`로 방향 인코딩 키 배열 생성):

```rust
/// 인메모리 줄 배열을 다중 기준으로 정렬해 데이터 행(data_start..)의 permutation을
/// 반환한다. permutation[i] = 정렬 순서 i번째로 올 원본 논리 행번호.
/// mmap 경로(extract_and_multi_sort)와 동일한 키 인코딩을 쓴다.
pub fn sort_lines(lines: &[String], specs: &[SortSpec], delim: u8, data_start: usize) -> Vec<u32> {
    if lines.len() <= data_start || specs.is_empty() {
        return Vec::new();
    }
    let data_rows = lines.len() - data_start;
    let mut keyed: Vec<([u64; MAX_KEYS], u32)> = (0..data_rows)
        .into_par_iter()
        .map(|i| {
            let logical = data_start + i;
            let line = lines[logical].as_bytes();
            let mut keys = [0u64; MAX_KEYS];
            for (slot, spec) in keys.iter_mut().zip(specs.iter()) {
                let field = parse::field_slice(line, delim, spec.col);
                *slot = col_key(field, *spec);
            }
            (keys, logical as u32)
        })
        .collect();
    keyed.par_sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, idx)| idx).collect()
}
```

- [ ] **Step 5: sort_lines 테스트 통과 확인**

Run: `cargo test sort::tests`
Expected: PASS(기존 정렬 테스트 + 새 3개).

- [ ] **Step 6: apply_permutation 실패 테스트**

`src/edit.rs` tests에 추가:

```rust
    #[test]
    fn apply_permutation_reorders_data_rows() {
        // order = [2,1] (논리 행번호), data_start=1 → 헤더 유지, 데이터 재배치.
        let mut lines = v(&["name", "banana", "apple"]);
        apply_permutation(&mut lines, &[2, 1], 1);
        assert_eq!(lines, v(&["name", "apple", "banana"]));
    }

    #[test]
    fn apply_permutation_no_header() {
        let mut lines = v(&["b", "a", "c"]);
        apply_permutation(&mut lines, &[1, 0, 2], 0);
        assert_eq!(lines, v(&["a", "b", "c"]));
    }
```

- [ ] **Step 7: apply_permutation 구현**

`src/edit.rs`에 추가:

```rust
/// order(원본 논리 행번호 배열) 순서로 데이터 행을 재배치한다. data_start 이전
/// 행(헤더)은 그대로 앞에 유지한다. order는 데이터 행만 커버한다.
pub fn apply_permutation(lines: &mut Vec<String>, order: &[u32], data_start: usize) {
    if data_start > lines.len() {
        return;
    }
    let header: Vec<String> = lines[..data_start].to_vec();
    let mut reordered: Vec<String> = Vec::with_capacity(order.len());
    for &idx in order {
        let i = idx as usize;
        if i < lines.len() {
            reordered.push(lines[i].clone());
        }
    }
    let mut out = header;
    out.append(&mut reordered);
    *lines = out;
}
```

- [ ] **Step 8: 테스트 통과 확인**

Run: `cargo test`
Expected: PASS (전체).

- [ ] **Step 9: 커밋**

```bash
git add src/sort.rs src/edit.rs
git commit -m "feat: 인메모리 정렬 - sort_lines + apply_permutation(lines 재배치)"
```

---

### Task 6: 저장 스트리밍 (인코딩 변환 + 원자적 rename)

**Files:**
- Create: `src/save.rs`
- Modify: `src/main.rs` (`mod save;`)
- Test: `src/save.rs`

**Interfaces:**
- Consumes: `crate::edit::Newline`, `crate::parse::Encoding`.
- Produces:
  - `pub struct SaveOptions { pub enc: crate::parse::Encoding, pub bom: bool, pub newline: crate::edit::Newline }`
  - `pub fn write_file(path: &Path, lines: &[String], opts: &SaveOptions, progress: Option<&dyn Fn(usize)>) -> std::io::Result<()>`
  - `pub fn encode_bytes(s: &str, enc: crate::parse::Encoding) -> Vec<u8>` (재인코딩 헬퍼)

- [ ] **Step 1: main.rs 모듈 선언**

`src/main.rs`에 `mod save;` 추가(`mod edit;` 아래).

- [ ] **Step 2: 실패 테스트**

`src/save.rs`를 만들고:

```rust
use crate::edit::Newline;
use crate::parse::Encoding;
use std::io::Write;
use std::path::Path;

pub struct SaveOptions {
    pub enc: Encoding,
    pub bom: bool,
    pub newline: Newline,
}

/// 문자열을 대상 인코딩 바이트로 변환한다.
pub fn encode_bytes(s: &str, enc: Encoding) -> Vec<u8> {
    todo!()
}

/// lines를 대상 인코딩/개행으로 임시 파일에 쓴 뒤 path로 원자적 rename 한다.
pub fn write_file(
    path: &Path,
    lines: &[String],
    opts: &SaveOptions,
    progress: Option<&dyn Fn(usize)>,
) -> std::io::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let id = C.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_save_{}_{}_{}.txt", std::process::id(), id, name));
        p
    }

    fn v(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn write_utf8_lf() {
        let p = tmp_path("u8");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: false, newline: Newline::Lf };
        write_file(&p, &v(&["a", "b"]), &opts, None).unwrap();
        let got = std::fs::read(&p).unwrap();
        assert_eq!(got, b"a\nb\n");
    }

    #[test]
    fn write_crlf() {
        let p = tmp_path("crlf");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: false, newline: Newline::CrLf };
        write_file(&p, &v(&["a", "b"]), &opts, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nb\r\n");
    }

    #[test]
    fn write_utf8_bom() {
        let p = tmp_path("bom");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: true, newline: Newline::Lf };
        write_file(&p, &v(&["x"]), &opts, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"\xEF\xBB\xBFx\n");
    }

    #[test]
    fn write_cp949_roundtrip() {
        let p = tmp_path("cp949");
        let opts = SaveOptions { enc: Encoding::Cp949, bom: false, newline: Newline::Lf };
        write_file(&p, &v(&["가나"]), &opts, None).unwrap();
        // CP949 "가나" = B0 A1 B3 AA
        assert_eq!(std::fs::read(&p).unwrap(), vec![0xB0, 0xA1, 0xB3, 0xAA, b'\n']);
    }

    #[test]
    fn write_utf16le_bom() {
        let p = tmp_path("u16le");
        let opts = SaveOptions { enc: Encoding::Utf16Le, bom: true, newline: Newline::Lf };
        write_file(&p, &v(&["A"]), &opts, None).unwrap();
        // BOM FF FE + 'A'(41 00) + '\n'(0A 00)
        assert_eq!(std::fs::read(&p).unwrap(), vec![0xFF, 0xFE, 0x41, 0x00, 0x0A, 0x00]);
    }

    #[test]
    fn encode_utf16be_char() {
        // 'A' in UTF-16BE = 00 41
        assert_eq!(encode_bytes("A", Encoding::Utf16Be), vec![0x00, 0x41]);
    }
}
```

- [ ] **Step 3: 실패 확인**

Run: `cargo test save::tests`
Expected: FAIL — todo!() 패닉.

- [ ] **Step 4: encode_bytes / write_file 구현**

먼저 `src/save.rs` 파일 상단의 `use` 목록에 encoding_rs를 추가한다(기존 `use crate::edit::Newline;` 등과 함께, 함수 밖 파일 최상단):

```rust
use encoding_rs::EUC_KR;
```

(UTF-16은 표준 라이브러리 `encode_utf16()`로 직접 처리하므로 EUC_KR만 필요.)

이어서 함수 구현:

```rust
pub fn encode_bytes(s: &str, enc: Encoding) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => s.as_bytes().to_vec(),
        Encoding::Cp949 => {
            let (cow, _, _) = EUC_KR.encode(s);
            cow.into_owned()
        }
        Encoding::Utf16Le => {
            let mut out = Vec::with_capacity(s.len() * 2);
            for u in s.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        Encoding::Utf16Be => {
            let mut out = Vec::with_capacity(s.len() * 2);
            for u in s.encode_utf16() {
                out.extend_from_slice(&u.to_be_bytes());
            }
            out
        }
    }
}

fn bom_bytes(enc: Encoding) -> &'static [u8] {
    match enc {
        Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoding::Utf16Le => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
        Encoding::Cp949 => &[],
    }
}

pub fn write_file(
    path: &Path,
    lines: &[String],
    opts: &SaveOptions,
    progress: Option<&dyn Fn(usize)>,
) -> std::io::Result<()> {
    let tmp = {
        let mut t = path.to_path_buf();
        let name = t.file_name().map(|n| n.to_owned()).unwrap_or_default();
        let mut n = name;
        n.push(".tmp");
        t.set_file_name(n);
        t
    };
    // 개행도 대상 인코딩으로 재인코딩(UTF-16은 개행이 2바이트).
    let nl_encoded = encode_bytes(
        match opts.newline {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
        },
        opts.enc,
    );

    let result = (|| -> std::io::Result<()> {
        let f = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::new(f);
        if opts.bom {
            w.write_all(bom_bytes(opts.enc))?;
        }
        for (i, line) in lines.iter().enumerate() {
            w.write_all(&encode_bytes(line, opts.enc))?;
            w.write_all(&nl_encoded)?;
            if let Some(p) = progress {
                if i % 65536 == 0 {
                    p(i);
                }
            }
        }
        w.flush()?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
```

주의: `use crate::edit::Newline;`가 파일 상단에 이미 있으므로 매치 확인. `Newline` variant 매칭에 `Newline::Lf`/`Newline::CrLf` 사용.

- [ ] **Step 5: 테스트 통과 확인**

Run: `cargo test save::tests`
Expected: PASS (6 tests).

- [ ] **Step 6: 커밋**

```bash
git add src/save.rs src/main.rs
git commit -m "feat: 저장 스트리밍 - 인코딩 변환·BOM·CRLF, 임시파일 원자적 rename"
```

---

### Task 7: Document 편집 상태 + 편집 모드 진입/이탈 (UI 배선 최소)

**Files:**
- Modify: `src/app.rs`
- Test: `src/app.rs`

**Interfaces:**
- Consumes: `crate::edit::{EditBuffer, load_edit_buffer}`.
- Produces:
  - `Document`에 필드 추가: `pub edit: Option<crate::edit::EditBuffer>`, 편집 UI 상태(아래).
  - `App`에 편집/저장 관련 다이얼로그 상태.
  - `logical_line(doc, logical) -> Option<String>` 헬퍼: 편집 모드면 `edit.lines[logical]`, 아니면 기존 `decode_logical_line`.

- [ ] **Step 1: Document/App 필드 추가**

`src/app.rs`의 `Document` struct에 필드 추가(기존 `sort_specs` 뒤):

```rust
    /// 편집 모드 인메모리 버퍼. None이면 뷰 전용(mmap).
    pub edit: Option<crate::edit::EditBuffer>,
    /// 셀 편집 중인 위치와 편집 텍스트(표 모드).
    pub editing_cell: Option<(usize, usize)>,
    pub cell_edit_text: String,
    /// 셀 사각 선택(표 모드): (r0,c0,r1,c1) 논리 행/열.
    pub cell_sel: Option<(usize, usize, usize, usize)>,
    /// 텍스트 선택(텍스트 모드): (anchor, caret).
    pub text_sel: Option<(crate::edit::TextPos, crate::edit::TextPos)>,
    /// 텍스트 커서(텍스트 모드).
    pub text_caret: crate::edit::TextPos,
```

`Document`를 생성하는 `open_path`의 `Some(Document { ... })` 리터럴에 위 필드 초기값 추가:

```rust
            edit: None,
            editing_cell: None,
            cell_edit_text: String::new(),
            cell_sel: None,
            text_sel: None,
            text_caret: crate::edit::TextPos { line: 0, col: 0 },
```

`App` struct에 저장 다이얼로그 상태 추가:

```rust
    /// 저장 다이얼로그 표시 + 편집 대상 인코딩/BOM 선택 상태.
    pub show_save_dialog: bool,
    pub save_as: bool,
    pub save_enc: crate::parse::Encoding,
    pub save_bom: bool,
```

`impl Default for App`에 초기값 추가:

```rust
            show_save_dialog: false,
            save_as: false,
            save_enc: crate::parse::Encoding::Utf8,
            save_bom: false,
```

- [ ] **Step 2: logical_line 헬퍼 + enter/exit 실패 테스트**

`src/app.rs` tests 모듈에 추가:

```rust
    #[test]
    fn enter_edit_mode_loads_lines() {
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        enter_edit_mode(doc);
        assert!(doc.edit.is_some());
        assert_eq!(doc.edit.as_ref().unwrap().lines, vec!["a,b", "1,2"]);
    }

    #[test]
    fn logical_line_reads_from_edit_buffer() {
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        enter_edit_mode(doc);
        // 편집 버퍼 값을 바꾸면 logical_line도 그 값을 반환.
        doc.edit.as_mut().unwrap().lines[1] = "X,Y".to_string();
        assert_eq!(logical_line(doc, 1).as_deref(), Some("X,Y"));
    }
```

- [ ] **Step 3: 실패 확인**

Run: `cargo test app::tests::enter_edit_mode app::tests::logical_line`
Expected: FAIL — 함수 미정의.

- [ ] **Step 4: enter_edit_mode / exit / logical_line 구현**

`src/app.rs`에 자유 함수로 추가(render 함수들 근처):

```rust
/// 편집 모드로 진입: 파일 전체를 현재 인코딩으로 줄 배열 로드.
/// (동기 로드 — 큰 파일 백그라운드화는 Task 9에서.)
pub fn enter_edit_mode(doc: &mut Document) {
    if doc.edit.is_some() {
        return;
    }
    let buf = crate::edit::load_edit_buffer(&doc.source, doc.enc);
    doc.edit = Some(buf);
    // 편집 모드에선 뷰 permutation 정렬을 폐기(이제 lines가 진실).
    doc.sort = None;
    doc.sort_job = None;
    doc.editing_cell = None;
    doc.cell_sel = None;
    doc.text_sel = None;
    doc.text_caret = crate::edit::TextPos { line: 0, col: 0 };
}

/// 편집 모드 이탈(버퍼 폐기). dirty 경고는 호출측 UI에서.
pub fn exit_edit_mode(doc: &mut Document) {
    doc.edit = None;
    doc.editing_cell = None;
    doc.cell_sel = None;
    doc.text_sel = None;
}

/// logical 논리 행의 텍스트. 편집 모드면 EditBuffer에서, 아니면 mmap 디코딩.
pub fn logical_line(doc: &Document, logical: usize) -> Option<String> {
    if let Some(e) = &doc.edit {
        e.lines.get(logical).cloned()
    } else {
        decode_logical_line(doc, logical)
    }
}
```

- [ ] **Step 5: 테스트 통과 확인**

Run: `cargo test app::tests`
Expected: PASS.

- [ ] **Step 6: 편집 모드 렌더 라인 수 배선**

`render_table`/`render_text`에서 총 행 수를 편집 모드면 `edit.lines.len()`으로 쓰도록 배선한다. 두 함수 상단의 `let total_lines = doc.index.line_count();`를 다음으로 교체:

```rust
    let total_lines = match &doc.edit {
        Some(e) => e.lines.len(),
        None => doc.index.line_count(),
    };
```

그리고 두 함수에서 줄 텍스트를 읽는 부분을 `logical_line(doc, logical)` 경유로 바꾼다:
- `render_text`: `decode_logical_line(doc, logical).unwrap_or_default()` → `logical_line(doc, logical).unwrap_or_default()`
- `render_table`: `parse_logical_line`이 내부에서 `decode_logical_line`을 쓰므로, 편집 모드 대응 버전 `parse_logical_line_edit(doc, logical, delim)`를 만들어 `logical_line`을 쓰게 한다:

```rust
fn parse_logical_line_edit(doc: &Document, logical: usize, delim: u8) -> Option<Vec<String>> {
    logical_line(doc, logical).map(|t| crate::parse::split_fields(&t, delim))
}
```

`render_table` 본문에서 `parse_logical_line(doc, l, delim)` 호출을 `parse_logical_line_edit(doc, l, delim)`로 교체(헤더/col_count 샘플/데이터 셀 3곳). 편집 아님일 때도 동일 결과이므로 뷰 모드 회귀 없음.

- [ ] **Step 7: 빌드 + 기존 테스트 통과 확인**

Run: `cargo build && cargo test`
Expected: 컴파일 성공, 전체 PASS(편집 모드 진입 없이는 뷰 동작 동일).

- [ ] **Step 8: 커밋**

```bash
git add src/app.rs
git commit -m "feat: Document 편집 상태 + enter/exit_edit_mode + logical_line 배선"
```

---

### Task 8: 편집 UI — 셀 편집·드래그 선택·우클릭 메뉴 (표 모드)

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: Task 3/4/7 함수(`set_cell`/`clear_cells`/`cells_to_tsv`/`paste_tsv`/`insert_row`/`remove_row`), `doc.edit`, `doc.cell_sel`, `doc.editing_cell`.
- Produces: `render_table`가 편집 모드일 때 셀 편집/선택/컨텍스트 메뉴를 처리.

**Note:** 이 태스크는 GUI 상호작용이라 단위 테스트가 어렵다. 각 스텝 후 `cargo build`로 컴파일을 확인하고, 최종에 `cargo build --release`로 수동 GUI 확인(스텝 8)한다. 순수 로직은 이미 Task 3/4에서 테스트됨.

- [ ] **Step 1: 데이터 셀 상호작용 배선**

`render_table`의 데이터 셀 렌더 루프(각 `row.col(|ui| { ... })`)에서, 편집 모드(`doc.edit.is_some()`)일 때:
- 셀 rect에 `ui.interact(rect, id, Sense::click_and_drag())`로 응답을 만든다.
- 드래그 시작(`resp.drag_started()`): `cell_sel = Some((view_row, c, view_row, c))` 시작점 저장. (논리 행이 아니라 표시 행/열 기준 — 편집 모드는 permutation 없음이라 표시행=논리행-data_start.)
- 드래그 중(`resp.dragged()`): 끝점 갱신 `cell_sel.3/.4 = (view_row, c)`.
- 더블클릭(`resp.double_clicked()`): `editing_cell = Some((logical, c))`, `cell_edit_text = 현재 필드 값`.
- 선택 영역이면 파란 음영(기존 `selected_col` 음영 로직 재사용, 셀 범위로 확장).

셀 편집 중(`editing_cell == Some((logical,c))`)이면 Label 대신 `TextEdit::singleline(&mut doc.cell_edit_text)`를 그리고, Enter/포커스 아웃 시 `set_cell(&mut e.lines, logical, c, &text, delim)` 호출 후 `editing_cell=None`, `e.dirty=true`.

구현은 기존 `render_table` 데이터 셀 블록을 편집 분기로 감싼다. Cell 상호작용 결과는 클로저 밖으로 넘길 통로(`Cell<Option<...>>`)를 기존 `clicked_col` 패턴처럼 만든다.

- [ ] **Step 2: 컨텍스트 메뉴**

데이터 셀 응답에 `.context_menu(|ui| { ... })`를 붙인다. 우클릭 셀이 현재 선택 밖이면 그 셀을 단일 선택으로 만든 뒤 메뉴. 항목:
- "복사" → `cells_to_tsv(...)` 결과를 `ui.output_mut(|o| o.copied_text = ...)`.
- "잘라내기" → 복사 후 `clear_cells(...)`.
- "붙여넣기" → 클립보드 텍스트를 `paste_tsv(...)`. (클립보드 읽기: egui는 붙여넣기 이벤트로 들어오므로, 간단히는 `ui.input(|i| i.events)`에서 `Event::Paste`를 별도 처리하거나, 우클릭 붙여넣기는 최근 복사 텍스트를 앱 필드에 보관해 사용. 최소 구현: 앱에 `clipboard_cache: String` 필드를 두고 복사 시 저장, 붙여넣기 시 사용.)
- "셀 내용 지우기" → `clear_cells(...)`.
- 구분선.
- "위에 행 삽입" → `insert_row(&mut e.lines, logical, String::new())`.
- "아래에 행 삽입" → `insert_row(&mut e.lines, logical+1, String::new())`.
- "행 삭제" → 선택 행 범위 `remove_row`.

모든 변경 후 `e.dirty = true`.

**클립보드 캐시:** `App`에 `pub clipboard_cache: String` 추가(Default `String::new()`). 복사/잘라내기 시 시스템 클립보드(`o.copied_text`)와 `clipboard_cache` 양쪽에 넣고, 붙여넣기는 `clipboard_cache` 사용(시스템 클립보드 직접 읽기는 egui 제약이 있어 최소 구현은 앱 캐시).

- [ ] **Step 3: 선택 음영 렌더**

`cell_sel`이 있으면 데이터 셀 렌더에서 그 사각 범위 셀에 반투명 파란 음영(`Color32::from_rgba_unmultiplied(80,150,230,70)`, 기존 컬럼 선택과 동일 색)을 painter로 덧그린다.

- [ ] **Step 4: 컴파일 확인**

Run: `cargo build`
Expected: 성공(경고 허용, 에러 없음).

- [ ] **Step 5: 셀 편집 회귀 테스트(순수 로직 재확인)**

이 태스크는 GUI라 단위 테스트가 없지만, 셀 편집이 호출하는 `set_cell` 경로가 깨지지 않았는지 전체 테스트 재실행:

Run: `cargo test`
Expected: 기존 전체 PASS.

- [ ] **Step 6: 커밋**

```bash
git add src/app.rs
git commit -m "feat: 표 모드 편집 UI - 셀 편집·드래그 선택·우클릭 메뉴"
```

---

### Task 9: 편집 UI — 텍스트 모드 자유 편집·드래그·우클릭 (텍스트 모드)

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: Task 2 함수(`insert_char`/`insert_str`/`split_line`/`delete_range`/`backspace`/`normalize`), `doc.text_caret`, `doc.text_sel`.
- Produces: `render_text`가 편집 모드일 때 문자 편집/선택/컨텍스트 메뉴 처리.

**Note:** GUI 상호작용. 순수 편집 로직은 Task 2에서 테스트됨. 컴파일 + 수동 확인.

- [ ] **Step 1: 키 입력 처리**

`render_text`에서 편집 모드일 때 `ui.input(|i| ...)`로 이벤트를 읽어 처리:
- `Event::Text(t)`: 선택 있으면 먼저 `delete_range`, 그다음 `insert_str(lines, caret, t)`.
- `Key::Enter`: 선택 삭제 후 `split_line`.
- `Key::Backspace`: 선택 있으면 `delete_range`, 없으면 `backspace`.
- `Key::Delete`: 선택 있으면 `delete_range`, 없으면 caret 다음 문자 삭제.
- 방향키/Home/End: caret 이동. Shift 동반이면 `text_sel` anchor 유지하며 확장.
- `Event::Paste(s)`: 선택 삭제 후 `insert_str`.
- Ctrl+C/X: 선택 텍스트를 `lines`에서 추출해 클립보드+캐시. X는 이후 `delete_range`.
- Ctrl+A: 전체 선택(`text_sel = (origin, 마지막 줄 끝)`).

편집이 일어나면 `e.dirty = true`.

선택 텍스트 추출 헬퍼(app.rs 내부): `selection_text(lines, a, b) -> String` — 정규화 후 시작 줄 뒷부분 + 중간 줄들 + 끝 줄 앞부분을 `\n`으로 join. (delete_range와 대칭.)

- [ ] **Step 2: 커서/선택 렌더**

`render_text` 각 줄 렌더에서, 편집 모드면:
- caret이 그 줄이면 caret 위치(char→x px)에 세로 막대(painter)를.
- 선택 범위가 그 줄에 걸치면 그 구간을 반투명 음영으로.
- 텍스트 자체는 기존 Label. (문자 위치→픽셀은 egui `Fonts`로 근사; 최소 구현은 monospace 가정 폭 계산 또는 galley 사용.)

드래그: 줄 rect에서 `Sense::click_and_drag()`. 클릭 위치 x → char 인덱스 변환(galley의 `cursor_from_pos` 또는 근사). down=anchor, drag=caret, up=확정.

- [ ] **Step 3: 컨텍스트 메뉴**

줄 응답에 `.context_menu(|ui| ...)`: 잘라내기 / 복사 / 붙여넣기 / 삭제 / 전체 선택. 각 항목은 Step 1의 연산 재사용.

- [ ] **Step 4: 컴파일 확인**

Run: `cargo build`
Expected: 성공.

- [ ] **Step 5: 전체 테스트**

Run: `cargo test`
Expected: 기존 전체 PASS.

- [ ] **Step 6: 커밋**

```bash
git add src/app.rs
git commit -m "feat: 텍스트 모드 자유 편집 UI - 키 입력·드래그 선택·우클릭 메뉴"
```

---

### Task 10: 편집 모드 정렬 재배치 + 메뉴·저장 다이얼로그 배선

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `crate::sort::sort_lines`, `crate::edit::apply_permutation`, `crate::save::{SaveOptions, write_file}`, `rfd`.
- Produces: 메뉴바 "편집 모드" 토글, "저장"/"다른 이름으로 저장…", 편집 모드 정렬 = lines 재배치, 저장 다이얼로그.

**Note:** GUI 배선. 순수 함수는 이미 테스트됨.

- [ ] **Step 1: 메뉴바에 편집/저장 항목 추가**

`update()`의 메뉴바에서:
- **파일** 메뉴: "열기…" 아래에 "저장"(`Ctrl+S`), "다른 이름으로 저장…" 추가. 편집 모드일 때만 활성(`doc.edit.is_some()`). 클릭 시 `show_save_dialog=true`, `save_as`는 각각 false/true.
- **도구** 메뉴: "편집 모드" 체크 항목. 체크 On → `enter_edit_mode(doc)`, Off → dirty면 경고 후 `exit_edit_mode`.

- [ ] **Step 2: 편집 모드 정렬 = 재배치**

`render_sort_controls`와 `render_sort_dialog`에서, 편집 모드(`doc.edit.is_some()`)면 spawn_sort/spawn_multi_sort(permutation) 대신:
- 단일 정렬 버튼: `let order = sort_lines(&e.lines, &[SortSpec{col,kind,dir,ci}], delim, data_start); apply_permutation(&mut e.lines, &order, data_start); e.dirty = true;`
- 다중 정렬 "정렬" 확정: `let order = sort_lines(&e.lines, &doc.sort_specs, delim, data_start); apply_permutation(...); e.dirty=true;`
- 편집 모드에선 `doc.sort`(SortState/화살표)는 설정하지 않는다(정렬이 이미 반영됨). 헤더 화살표 대신 상태바에 "정렬됨(편집)" 정도만.

큰 파일 대비: `sort_lines`는 인메모리라 빠르지만, 필요 시 백그라운드화는 후속. 최소 구현은 동기 호출(정렬 클릭 시 잠깐 멈춤 허용).

- [ ] **Step 3: 저장 다이얼로그 구현**

`render_save_dialog(ctx, app)` 함수 신설:
- `egui::Window`로 모달. 인코딩 ComboBox(UTF-8/CP949/UTF-16LE/BE), BOM 체크(CP949면 비활성), "저장"/"취소".
- "다른 이름으로 저장"이면 rfd `FileDialog::new().save_file()`로 경로 선택(다이얼로그 열 때 또는 저장 클릭 시).
- "저장"이면 현재 경로(`doc.path_label`) 사용. 경로가 없으면 save_as로 폴백.
- 확정 시 `write_file(&path, &e.lines, &SaveOptions{enc, bom, newline: e.newline}, None)`. 성공 시 `e.dirty=false`, save_as면 `path_label` 갱신. 실패 시 `app.error`.

`update()` 끝부분에 다이얼로그 렌더 호출 추가:

```rust
        if self.show_save_dialog {
            render_save_dialog(ctx, self);
        }
```

- [ ] **Step 4: dirty 경고**

편집 모드 이탈/다른 파일 열기 시 `e.dirty`면 확인 다이얼로그("저장하지 않은 변경이 있습니다. 계속하시겠습니까?"). 최소 구현은 간단한 egui 확인 창 또는 상태바 경고 후 진행. (완전한 모달 확인은 여력 되면.)

- [ ] **Step 5: 상태바 편집/dirty 표시**

상태바에 편집 모드면 "편집 중" + dirty면 "●(변경됨)" 표시.

- [ ] **Step 6: 빌드 + 전체 테스트**

Run: `cargo build --release && cargo test`
Expected: 컴파일 성공, 전체 PASS.

- [ ] **Step 7: 수동 GUI 확인**

`cargo run --release`로 실행해 다음을 확인(사용자와 함께):
1. 파일 열기 → 도구 → 편집 모드 On → 셀 더블클릭 편집 → 값 반영.
2. 셀 드래그 선택 → 우클릭 복사/붙여넣기/삭제.
3. 텍스트 파일 열기 → 편집 모드 → 문자 입력/Enter/Backspace/드래그.
4. 편집 모드에서 정렬 → 행이 재배치되고 그 상태에서 행 삽입 → 자리 유지.
5. 파일 → 다른 이름으로 저장 → 인코딩 CP949 선택 → 저장 → 다시 열어 확인.

- [ ] **Step 8: 커밋**

```bash
git add src/app.rs
git commit -m "feat: 편집 모드 정렬 재배치 + 메뉴·저장 다이얼로그(인코딩 변환)"
```

---

### Task 11: .readme 정리 + 최종 검증

**Files:**
- Create: `.readme/20260723_편집모드와_저장.md`

- [ ] **Step 1: .readme 문서 작성**

`.readme/20260723_편집모드와_저장.md`에 이번 작업을 정리(형식: 한 줄 요약 / 기능 / 설계 / 파일 구조 / 테스트 / 다음 단계). 참고: 기존 `.readme/20260723_*.md` 형식.

- [ ] **Step 2: 최종 전체 검증**

Run: `cargo test && cargo build --release`
Expected: 전체 PASS, 릴리스 빌드 경고 0(가능하면).

- [ ] **Step 3: 커밋**

```bash
git add .readme/20260723_편집모드와_저장.md
git commit -m "docs: 편집 모드 + 저장 작업 .readme 정리"
```

---

## 실행 후

모든 태스크 완료 후 `superpowers:finishing-a-development-branch`로 master 로컬 머지(origin push는 사용자 명시 지시 시에만).
