use crate::parse::{decode_line, Encoding};
use crate::source::Source;
use crate::parse::{join_fields, split_fields};

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
    /// 되돌리기 스택(최근 UNDO_LIMIT 단계).
    pub undo: UndoStack,
}

/// mmap 바이트를 개행(`\n`)으로 분할하고 각 줄을 `enc`로 디코딩해 EditBuffer를 만든다.
/// - `\r\n`이 하나라도 있으면 newline=CrLf, 아니면 Lf.
/// - 각 줄에서 뒤쪽 `\r`/`\n`을 제거한다.
/// - 마지막 바이트가 개행이면 그 뒤에 빈 줄을 추가하지 않는다(파일 끝 개행은 종결자).
/// - 빈 파일이면 `lines = [""]`(빈 한 줄).
pub fn load_edit_buffer(source: &Source, enc: Encoding) -> EditBuffer {
    let bytes = source.as_bytes();
    if bytes.is_empty() {
        return EditBuffer {
            lines: vec![String::new()],
            dirty: false,
            newline: Newline::Lf,
            undo: UndoStack::new(UNDO_LIMIT),
        };
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
    EditBuffer { lines, dirty: false, newline, undo: UndoStack::new(UNDO_LIMIT) }
}

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

/// [a, b) 범위의 텍스트를 추출한다(`delete_range`의 대칭). 정규화 후
/// 시작 줄 뒷부분 + 중간 줄 전체 + 끝 줄 앞부분을 `\n`으로 join한다.
/// 빈 범위(a == b)면 빈 문자열.
pub fn selection_text(lines: &[String], a: TextPos, b: TextPos) -> String {
    let (a, b) = normalize(a, b);
    if a == b || a.line >= lines.len() {
        return String::new();
    }
    if a.line == b.line {
        let s = &lines[a.line];
        let sa = byte_of(s, a.col);
        let sb = byte_of(s, b.col);
        return s[sa.min(sb)..sb.max(sa)].to_owned();
    }
    let mut out = String::new();
    // 시작 줄의 뒷부분.
    let sa = byte_of(&lines[a.line], a.col);
    out.push_str(&lines[a.line][sa..]);
    // 중간 줄 전체.
    let last = b.line.min(lines.len().saturating_sub(1));
    for l in (a.line + 1)..last {
        out.push('\n');
        out.push_str(&lines[l]);
    }
    // 끝 줄의 앞부분.
    if last > a.line {
        out.push('\n');
        let sb = byte_of(&lines[last], b.col);
        out.push_str(&lines[last][..sb]);
    }
    out
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

/// logical 행의 col번째 필드를 value로 교체하고 줄을 재조립한다.
/// col이 현재 필드 수보다 크면 빈 필드로 패딩한다.
/// 셀 값은 한 행에 속하므로 개행을 포함할 수 없다 — 개행은 공백으로 치환해
/// lines[i] 불변식(개행 없음)을 보장한다.
pub fn set_cell(lines: &mut [String], logical: usize, col: usize, value: &str, delim: u8) {
    let value = sanitize_cell_value(value);
    let mut fields = split_fields(&lines[logical], delim);
    if col >= fields.len() {
        fields.resize(col + 1, String::new());
    }
    fields[col] = value;
    lines[logical] = join_fields(&fields, delim);
}

/// 셀 값에 포함된 `\n`/`\r`를 공백으로 치환한다. 셀은 한 행에 속하므로
/// 개행을 담을 수 없다 — lines[i]에 개행이 박히는 것을 방지한다.
fn sanitize_cell_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

/// 사각 영역 [r0..=r1] x [c0..=c1]의 각 셀을 빈 값으로 만든다.
///
/// 성능 주의: 컬럼 선택은 데이터 전 구간(수억 행)을 범위로 만들 수 있다. 값을
/// 비우려면 줄을 재조립해야 하므로 `split_fields`+`join_fields`가 필요하지만,
/// **바꿀 게 없는 행은 통째로 건너뛴다** — 대상 필드가 이미 전부 비어 있으면
/// `field_slice`(할당 없음)만으로 판정하고 재조립을 생략한다.
pub fn clear_cells(lines: &mut [String], r0: usize, c0: usize, r1: usize, c1: usize, delim: u8) {
    let (r0, r1) = (r0.min(r1), r0.max(r1));
    let (c0, c1) = (c0.min(c1), c0.max(c1));
    for r in r0..=r1 {
        if r >= lines.len() {
            break;
        }
        // 이미 전부 비어 있으면 재조립(할당) 자체를 건너뛴다.
        let already_empty = {
            let bytes = lines[r].as_bytes();
            (c0..=c1)
                .all(|c| crate::parse::field_slice(bytes, delim, c).map_or(true, |f| f.is_empty()))
        };
        if already_empty {
            continue;
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

/// 선택 사각 영역을 TSV(행=\n, 열=\t)로 직렬화한다.
///
/// 성능 주의: 컬럼 선택(헤더 클릭)은 데이터 전 구간을 범위로 만든다 —
/// 3억 행이면 이 루프가 3억 번 돈다. 그래서 행마다 `split_fields`로
/// **모든** 필드를 `String`으로 만들지 않고(리더 생성 + Vec 2개 + 필드 수만큼
/// String 할당), 필요한 컬럼만 `parse::field_slice`로 **할당 없이** 잘라 온다.
/// `field_slice`는 raw 슬라이스(따옴표 포함)를 주므로 `unquote_field`로
/// `split_fields`와 같은 값이 되도록 인용을 벗긴다.
pub fn cells_to_tsv(lines: &[String], r0: usize, c0: usize, r1: usize, c1: usize, delim: u8) -> String {
    let (r0, r1) = (r0.min(r1), r0.max(r1));
    let (c0, c1) = (c0.min(c1), c0.max(c1));
    // 결과 크기를 대략 예측해 재할당을 줄인다(행당 평균 16바이트 가정).
    let mut out = String::with_capacity((r1.saturating_sub(r0) + 1).saturating_mul(16));
    for r in r0..=r1 {
        if r > r0 {
            out.push('\n');
        }
        if r >= lines.len() {
            continue;
        }
        let bytes = lines[r].as_bytes();
        for c in c0..=c1 {
            if c > c0 {
                out.push('\t');
            }
            // 할당 없이 해당 필드 슬라이스만. 따옴표로 감싼 값은 그대로
            // 포함되므로 여기서 벗겨 준다(split_fields와 결과를 맞추기 위함).
            if let Some(f) = crate::parse::field_slice(bytes, delim, c) {
                push_unquoted(&mut out, f);
            }
        }
    }
    out
}

/// `field_slice`가 돌려준 raw 필드에서 CSV 인용을 벗겨 `out`에 덧붙인다.
///
/// **csv_core(= `split_fields`)의 동작을 그대로 재현한다.** 브리프가 제안한
/// "앞뒤가 `"`면 벗기고 내부 `""`를 하나로" 규칙은 실제 csv_core와 다음
/// 입력들에서 어긋나므로 채택하지 않았다(모두 실측):
///   `"`      → csv_core `""`,      단순규칙 `"\""`   (len<2라 안 벗김)
///   `"ab`    → csv_core `"ab"`,    단순규칙 `"\"ab"` (끝이 따옴표가 아님)
///   `"a"b"`  → csv_core `"ab\""`,  단순규칙 `"a\"b"` (중간 따옴표에서 인용 종료)
///   `"a"b`   → csv_core `"ab"`,    단순규칙 그대로
/// 실제 규칙은 상태기계다:
///   - 필드가 `"`로 시작하지 않으면 **전부 그대로**(내부 `"`도 리터럴).
///   - `"`로 시작하면 인용 상태로 진입하고 그 여는 따옴표는 버린다.
///     인용 안에서 `""`는 `"` 하나로, 홀로 있는 `"`는 인용을 **닫는다**.
///     인용을 닫은 뒤의 바이트는 다시 리터럴이다(닫는 따옴표 자체는 버려짐).
/// 할당은 하지 않는다 — 바이트 구간을 그대로 `out`에 복사한다.
fn push_unquoted(out: &mut String, f: &[u8]) {
    if !f.starts_with(b"\"") {
        out.push_str(&String::from_utf8_lossy(f));
        return;
    }
    let mut i = 1usize; // 여는 따옴표는 버린다
    let mut in_quotes = true;
    let mut seg_start = i;
    while i < f.len() {
        if in_quotes && f[i] == b'"' {
            // 여기까지의 구간을 흘려보내고 따옴표를 처리한다.
            out.push_str(&String::from_utf8_lossy(&f[seg_start..i]));
            if f.get(i + 1) == Some(&b'"') {
                out.push('"'); // "" → " (인용 유지)
                i += 2;
            } else {
                in_quotes = false; // 인용 종료(닫는 따옴표는 버림)
                i += 1;
            }
            seg_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&String::from_utf8_lossy(&f[seg_start..]));
}

/// TSV 클립보드를 (r0,c0)부터 셀 그리드로 덮어쓴다. 경계를 넘으면 행/열 확장.
/// 붙여넣는 값은 set_cell과 동일하게 sanitize_cell_value를 거쳐 lines[i] 개행 불변식을 지킨다.
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
            fields[c] = sanitize_cell_value(cell);
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

/// 되돌리기 한 단계. 각 variant는 그 편집을 취소하는 데 필요한 최소 정보를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    /// 여러 줄의 이전 내용을 복원한다(셀 편집·범위 지우기·붙여넣기 등 값 변경).
    /// (논리 행번호, 그 행의 이전 전체 텍스트) 목록.
    Replace(Vec<(usize, String)>),
    /// 삽입된 행들을 제거한다. at부터 count개를 지우면 원상복구.
    RemoveInserted { at: usize, count: usize },
    /// 삭제된 행들을 되살린다. at 위치에 lines를 다시 끼워 넣는다.
    ReinsertRemoved { at: usize, lines: Vec<String> },
    /// 정렬 등 전체 재배치를 되돌린다. inverse[i] = 재배치 후 i번째 줄이
    /// 원래 있던 위치. data_start 이전(헤더)은 건드리지 않는다.
    Reorder { inverse: Vec<u32>, data_start: usize },
    /// 여러 op를 **한 단계**로 묶는다. 한 번의 사용자 동작이 구조 변화와 값
    /// 변화를 동시에 일으킬 때(예: 붙여넣기로 행이 늘어남, Enter로 줄이 갈라짐)
    /// Ctrl+Z 한 번에 전부 되돌아가야 하기 때문에 필요하다.
    /// 내부 op는 **앞에서부터 순서대로** 적용된다 — 즉 `Batch(vec![a, b])`는
    /// a를 먼저, b를 나중에 되돌린다(스택의 LIFO와 반대 방향이므로,
    /// 만드는 쪽이 "되돌릴 순서"대로 담으면 된다).
    Batch(Vec<EditOp>),
}

/// 되돌리기 스택. limit을 넘으면 가장 오래된 것부터 버린다(FIFO).
pub struct UndoStack {
    ops: Vec<EditOp>,
    limit: usize,
}

/// 기본 되돌리기 깊이(사용자 확정).
pub const UNDO_LIMIT: usize = 100;

impl UndoStack {
    pub fn new(limit: usize) -> Self {
        UndoStack { ops: Vec::new(), limit }
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// 되돌리기 한 단계를 쌓는다. limit을 넘으면 가장 오래된 것을 버린다.
    /// **편집을 적용하기 직전에** 호출해야 한다(이전 상태를 담으므로).
    pub fn push(&mut self, op: EditOp) {
        if self.limit == 0 {
            return;
        }
        if self.ops.len() >= self.limit {
            self.ops.remove(0);
        }
        self.ops.push(op);
    }

    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// 가장 최근 편집을 되돌린다. 되돌릴 게 없으면 false.
    /// 범위를 벗어난 인덱스는 조용히 건너뛴다(패닉 없음).
    pub fn undo(&mut self, lines: &mut Vec<String>) -> bool {
        let Some(op) = self.ops.pop() else {
            return false;
        };
        apply_undo_op(lines, op);
        // "lines는 비어 있지 않다" 불변식은 **한 단계가 끝난 뒤**에만 강제한다.
        // Batch 중간에 강제하면 유령 줄이 다시 생겨(제거 → 즉시 재삽입) 뒤이은
        // 되꽂기가 한 줄 늘어난 결과를 낳는다.
        if lines.is_empty() {
            lines.push(String::new());
        }
        true
    }
}

/// op 하나를 lines에 적용한다(되돌리기 실행). `Batch`는 담긴 순서대로 재귀 적용.
/// 빈 lines 방어는 호출측(`UndoStack::undo`)이 한 단계 끝에 한 번만 한다.
fn apply_undo_op(lines: &mut Vec<String>, op: EditOp) {
    match op {
        EditOp::Replace(items) => {
            for (row, text) in items {
                if row < lines.len() {
                    lines[row] = text;
                }
            }
        }
        EditOp::RemoveInserted { at, count } => {
            let end = (at + count).min(lines.len());
            if at < end {
                lines.drain(at..end);
            }
        }
        EditOp::ReinsertRemoved { at, lines: removed } => {
            let at = at.min(lines.len());
            for (k, text) in removed.into_iter().enumerate() {
                lines.insert(at + k, text);
            }
        }
        EditOp::Reorder { inverse, data_start } => {
            apply_permutation(lines, &inverse, data_start);
        }
        EditOp::Batch(ops) => {
            for inner in ops {
                apply_undo_op(lines, inner);
            }
        }
    }
}

/// 재배치 order의 역순열을 만든다. `apply_permutation(lines, order, ds)`를
/// 되돌리려면 `apply_permutation(lines, inverse_of(order, ds), ds)`를 적용하면 된다.
/// order[i] = 재배치 후 i번째 자리에 올 원본 논리 행번호.
pub fn inverse_of(order: &[u32], data_start: usize) -> Vec<u32> {
    let mut inv = vec![0u32; order.len()];
    for (new_i, &orig) in order.iter().enumerate() {
        let orig_slot = (orig as usize).saturating_sub(data_start);
        if orig_slot < inv.len() {
            inv[orig_slot] = (data_start + new_i) as u32;
        }
    }
    inv
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
    fn selection_text_single_line() {
        let lines = v(&["abcdef"]);
        let s = selection_text(
            &lines,
            TextPos { line: 0, col: 1 },
            TextPos { line: 0, col: 4 },
        );
        assert_eq!(s, "bcd");
    }

    #[test]
    fn selection_text_multiline() {
        // delete_range_multiline과 같은 범위 — 지워지는 부분이 그대로 나와야 한다.
        let lines = v(&["abc", "XXX", "defg"]);
        let s = selection_text(
            &lines,
            TextPos { line: 0, col: 1 },
            TextPos { line: 2, col: 2 },
        );
        assert_eq!(s, "bc\nXXX\nde");
    }

    #[test]
    fn selection_text_empty_range() {
        let lines = v(&["abc"]);
        let s = selection_text(
            &lines,
            TextPos { line: 0, col: 2 },
            TextPos { line: 0, col: 2 },
        );
        assert_eq!(s, "");
    }

    #[test]
    fn selection_text_reversed_is_normalized() {
        let lines = v(&["ab", "cd"]);
        let s = selection_text(
            &lines,
            TextPos { line: 1, col: 1 },
            TextPos { line: 0, col: 1 },
        );
        assert_eq!(s, "b\nc");
    }

    #[test]
    fn selection_text_two_adjacent_lines() {
        let lines = v(&["ab", "cd", "ef"]);
        let s = selection_text(
            &lines,
            TextPos { line: 0, col: 2 },
            TextPos { line: 1, col: 0 },
        );
        // 시작 줄 끝 ~ 다음 줄 처음 = 개행 하나.
        assert_eq!(s, "\n");
    }

    #[test]
    fn selection_text_multibyte_chars() {
        // col은 char 인덱스 — 바이트 인덱스로 자르면 깨진다.
        let lines = v(&["가나다라"]);
        let s = selection_text(
            &lines,
            TextPos { line: 0, col: 1 },
            TextPos { line: 0, col: 3 },
        );
        assert_eq!(s, "나다");
    }

    #[test]
    fn selection_text_mirrors_delete_range() {
        // selection_text가 뽑아낸 것을 다시 넣으면 원래대로 — 잘라내기/붙여넣기 왕복.
        let orig = v(&["hello", "world", "again"]);
        let a = TextPos { line: 0, col: 2 };
        let b = TextPos { line: 2, col: 3 };
        let cut = selection_text(&orig, a, b);
        let mut lines = orig.clone();
        let p = delete_range(&mut lines, a, b);
        let end = insert_str(&mut lines, p, &cut);
        assert_eq!(lines, orig);
        assert_eq!(end, b);
    }

    #[test]
    fn normalize_swaps_when_reversed() {
        let a = TextPos { line: 2, col: 0 };
        let b = TextPos { line: 1, col: 3 };
        assert_eq!(normalize(a, b), (b, a));
    }

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
    fn set_cell_strips_newlines_from_value() {
        // 셀 값에 개행이 있으면 공백으로 치환 — lines[i]에 개행이 박히지 않아야 한다.
        let mut lines = v(&["a,b"]);
        set_cell(&mut lines, 0, 0, "x\ny", b',');
        assert_eq!(lines, v(&["x y,b"]));
        assert!(!lines[0].contains('\n'));
    }

    #[test]
    fn set_cell_strips_carriage_return() {
        let mut lines = v(&["a,b"]);
        set_cell(&mut lines, 0, 1, "p\r\nq", b',');
        // \r\n 두 문자가 각각 공백으로 → "p  q"
        assert_eq!(lines, v(&["a,p  q"]));
    }

    #[test]
    fn clear_cells_rectangle() {
        // 2x2 영역(행0~1, 열0~1)을 빈 값으로.
        let mut lines = v(&["a,b,c", "d,e,f", "g,h,i"]);
        clear_cells(&mut lines, 0, 0, 1, 1, b',');
        assert_eq!(lines, v(&[",,c", ",,f", "g,h,i"]));
    }

    /// 이미 비어 있는 대상 행은 재조립을 건너뛴다 — 그 결과 줄이 **바이트
    /// 단위로 그대로** 남아야 한다. app.rs의 Cut/Clear no-op undo 판정
    /// (`edit_op_differs_from_current`)이 "안 바뀜"을 정확히 보려면 이 성질이
    /// 필요하다. 특히 재조립(split+join)은 인용을 정규화해 버려서 값이
    /// 같더라도 줄 텍스트가 달라질 수 있다.
    #[test]
    fn clear_cells_skips_already_empty_rows_byte_exactly() {
        // col 1이 이미 비어 있고, col 0은 재조립되면 표현이 달라질 수 있는 값.
        let mut lines = v(&["\"a\"b,,c", ",,", "x,y,z"]);
        let before = lines.clone();
        clear_cells(&mut lines, 0, 1, 1, 1, b',');
        assert_eq!(lines[0], before[0], "이미 빈 행은 손대지 않는다");
        assert_eq!(lines[1], before[1], "이미 빈 행은 손대지 않는다");
        assert_eq!(lines[2], before[2], "범위 밖 행은 그대로");
    }

    #[test]
    fn clear_cells_still_clears_non_empty() {
        // 건너뛰기 최적화가 실제로 지워야 할 행까지 건너뛰면 안 된다.
        let mut lines = v(&["a,b,c", "d,,f", "g,h,i"]);
        clear_cells(&mut lines, 0, 1, 2, 1, b',');
        assert_eq!(lines, v(&["a,,c", "d,,f", "g,,i"]));
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

    #[test]
    fn cells_to_tsv_basic() {
        let lines = v(&["a,b,c", "d,e,f"]);
        // 열 0~1, 행 0~1 → "a\tb\nd\te"
        let s = cells_to_tsv(&lines, 0, 0, 1, 1, b',');
        assert_eq!(s, "a\tb\nd\te");
    }

    #[test]
    fn cells_to_tsv_single_column_matches_split_fields() {
        // 최적화 경로(단일 컬럼)와 일반 경로가 같은 결과를 내야 한다.
        let lines = v(&["a,b,c", "d,e,f", "\"x,y\",z,w"]);
        for col in 0..3 {
            let fast = cells_to_tsv(&lines, 0, col, 2, col, b',');
            // 일반 경로를 직접 재현(참조 구현).
            let mut want = String::new();
            for (i, l) in lines.iter().enumerate() {
                if i > 0 {
                    want.push('\n');
                }
                let f = crate::parse::split_fields(l, b',');
                want.push_str(f.get(col).map(|s| s.as_str()).unwrap_or(""));
            }
            assert_eq!(fast, want, "col {col}");
        }
    }

    /// `unquote_field`가 `split_fields`의 인용 해제 의미를 정확히 재현하는지.
    /// 감싸는 따옴표 제거 + 내부 `""` → `"` 복원, 인용이 아닌 값은 그대로.
    #[test]
    fn cells_to_tsv_matches_split_fields_on_tricky_quoting() {
        let lines = v(&[
            "\"a\"\"b\",z",       // 내부 이스케이프된 따옴표
            "\"\",z",             // 빈 인용 필드
            ",z",                 // 빈 필드(인용 아님)
            "\"only\",z",         // 평범한 인용
            "\"\"\"\",z",         // 인용 안의 따옴표 하나
        ]);
        for col in 0..2 {
            let fast = cells_to_tsv(&lines, 0, col, lines.len() - 1, col, b',');
            let mut want = String::new();
            for (i, l) in lines.iter().enumerate() {
                if i > 0 {
                    want.push('\n');
                }
                let f = crate::parse::split_fields(l, b',');
                want.push_str(f.get(col).map(|s| s.as_str()).unwrap_or(""));
            }
            assert_eq!(fast, want, "col {col}");
        }
    }

    /// `{a, ", ,}` 알파벳으로 만든 길이 0~5의 **모든** 문자열에 대해
    /// `cells_to_tsv`(field_slice + 인용 해제)가 `split_fields` 경로와
    /// 완전히 같은 값을 내는지 전수 검사한다. 브리프의 단순 인용 해제
    /// 규칙이 어긋났던 케이스(`"`, `"ab`, `"a"b"` 등)가 모두 여기 포함된다.
    #[test]
    fn cells_to_tsv_matches_split_fields_exhaustively() {
        const ALPHABET: [char; 3] = ['a', '"', ','];
        let mut checked = 0usize;
        for len in 0..=5usize {
            let total = ALPHABET.len().pow(len as u32);
            for n in 0..total {
                let mut n = n;
                let mut s = String::new();
                for _ in 0..len {
                    s.push(ALPHABET[n % ALPHABET.len()]);
                    n /= ALPHABET.len();
                }
                let lines = vec![s.clone()];
                let want = crate::parse::split_fields(&s, b',');
                for col in 0..want.len() {
                    let got = cells_to_tsv(&lines, 0, col, 0, col, b',');
                    assert_eq!(got, want[col], "input {s:?} col {col}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 500, "전수 검사가 실제로 돌았는지 확인");
    }

    /// 대용량 컬럼 복사 벤치. 무시 상태이며 필요할 때만 돌린다:
    ///   cargo test --release bench_column_copy -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_column_copy() {
        use std::time::Instant;
        let rows = 2_000_000;
        let lines: Vec<String> = (0..rows).map(|i| format!("{i},bbb,ccc,ddd")).collect();
        let t = Instant::now();
        let s = cells_to_tsv(&lines, 0, 1, rows - 1, 1, b',');
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("컬럼 복사 {rows}행: {ms:.0}ms, 결과 {}바이트", s.len());
    }

    /// 대용량 컬럼 지우기 벤치(clear_cells). 이미 비어 있는 행은 재조립을
    /// 건너뛴다는 것을 확인하기 위해 "이미 빈 컬럼" 케이스를 함께 잰다.
    ///   cargo test --release bench_column_clear -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_column_clear() {
        use std::time::Instant;
        let rows = 2_000_000;
        let mut lines: Vec<String> = (0..rows).map(|i| format!("{i},bbb,ccc,ddd")).collect();
        let t = Instant::now();
        clear_cells(&mut lines, 0, 1, rows - 1, 1, b',');
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("컬럼 지우기(값 있음) {rows}행: {ms:.0}ms");
        // 두 번째 호출: 이미 전부 비어 있으므로 재조립을 건너뛰어야 한다.
        let t = Instant::now();
        clear_cells(&mut lines, 0, 1, rows - 1, 1, b',');
        let ms2 = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("컬럼 지우기(이미 빔) {rows}행: {ms2:.0}ms");
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

    #[test]
    fn paste_tsv_sanitizes_carriage_return() {
        // CRLF 클립보드: \n으로 split 후 남는 \r이 셀에 박히면 안 된다.
        let mut lines = v(&["a,b"]);
        paste_tsv(&mut lines, 0, 0, "X\r\nY", b',');
        assert!(!lines.iter().any(|l| l.contains('\r')));
        assert!(!lines.iter().any(|l| l.contains('\n')));
    }

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

    #[test]
    fn undo_replace_restores_previous_lines() {
        let mut lines = v(&["a,b", "c,d"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        // 1행을 바꾸기 전에 이전 값을 기록.
        st.push(EditOp::Replace(vec![(1, lines[1].clone())]));
        lines[1] = "X,Y".to_string();
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["a,b", "c,d"]));
    }

    #[test]
    fn undo_remove_inserted_rows() {
        let mut lines = v(&["a", "b"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        insert_row(&mut lines, 1, String::new());
        st.push(EditOp::RemoveInserted { at: 1, count: 1 });
        assert_eq!(lines, v(&["a", "", "b"]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["a", "b"]));
    }

    #[test]
    fn undo_reinsert_removed_rows() {
        let mut lines = v(&["a", "b", "c"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::ReinsertRemoved { at: 1, lines: vec!["b".to_string()] });
        remove_row(&mut lines, 1);
        assert_eq!(lines, v(&["a", "c"]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["a", "b", "c"]));
    }

    #[test]
    fn undo_reorder_restores_original_order() {
        // 정렬로 재배치한 뒤 undo하면 원래 순서로 돌아온다(헤더 유지).
        let mut lines = v(&["hdr", "b", "a", "c"]);
        let order: Vec<u32> = vec![2, 1, 3]; // a(2), b(1), c(3)
        let inverse = inverse_of(&order, 1);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::Reorder { inverse, data_start: 1 });
        apply_permutation(&mut lines, &order, 1);
        assert_eq!(lines, v(&["hdr", "a", "b", "c"]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["hdr", "b", "a", "c"]));
    }

    #[test]
    fn undo_pops_in_lifo_order() {
        let mut lines = v(&["a"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::Replace(vec![(0, "a".to_string())]));
        lines[0] = "b".to_string();
        st.push(EditOp::Replace(vec![(0, "b".to_string())]));
        lines[0] = "c".to_string();
        st.undo(&mut lines);
        assert_eq!(lines, v(&["b"]), "최근 것부터 되돌린다");
        st.undo(&mut lines);
        assert_eq!(lines, v(&["a"]));
    }

    #[test]
    fn undo_on_empty_stack_is_false() {
        let mut lines = v(&["a"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        assert!(!st.undo(&mut lines));
        assert_eq!(lines, v(&["a"]));
    }

    #[test]
    fn undo_stack_drops_oldest_beyond_limit() {
        let mut st = UndoStack::new(2);
        st.push(EditOp::Replace(vec![(0, "1".into())]));
        st.push(EditOp::Replace(vec![(0, "2".into())]));
        st.push(EditOp::Replace(vec![(0, "3".into())]));
        assert_eq!(st.len(), 2, "가장 오래된 것이 버려진다");
    }

    #[test]
    fn undo_replace_out_of_range_row_is_ignored() {
        // 행이 줄어든 뒤 낡은 인덱스가 들어와도 패닉하지 않는다.
        let mut lines = v(&["a"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::Replace(vec![(5, "x".to_string())]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["a"]));
    }

    #[test]
    fn undo_batch_is_one_step() {
        // 붙여넣기로 행이 늘어난 상황: 덮어쓴 행 복원 + 늘어난 행 제거가
        // Ctrl+Z 한 번에 모두 일어나야 한다.
        let mut lines = v(&["a,b"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::Batch(vec![
            EditOp::RemoveInserted { at: 1, count: 1 },
            EditOp::Replace(vec![(0, "a,b".to_string())]),
        ]));
        paste_tsv(&mut lines, 0, 0, "1\t2\n3\t4", b',');
        assert_eq!(lines, v(&["1,2", "3,4"]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["a,b"]), "한 번의 undo로 완전 복구");
        assert!(st.is_empty(), "Batch는 한 단계만 소비한다");
    }

    #[test]
    fn undo_batch_applies_in_order() {
        // 담긴 순서대로 적용된다: 먼저 유령 줄 제거 → 그다음 원본 되꽂기.
        let mut lines = v(&["only"]);
        let mut st = UndoStack::new(UNDO_LIMIT);
        st.push(EditOp::Batch(vec![
            EditOp::RemoveInserted { at: 0, count: 1 },
            EditOp::ReinsertRemoved { at: 0, lines: v(&["x", "y"]) },
        ]));
        // 전부 삭제 → remove_row가 빈 한 줄을 남긴다.
        remove_row(&mut lines, 0);
        assert_eq!(lines, v(&[""]));
        assert!(st.undo(&mut lines));
        assert_eq!(lines, v(&["x", "y"]), "유령 줄 없이 정확히 복원");
    }

    #[test]
    fn inverse_of_roundtrips() {
        let order: Vec<u32> = vec![3, 1, 2];
        let inv = inverse_of(&order, 1);
        let mut lines = v(&["h", "x", "y", "z"]);
        let before = lines.clone();
        apply_permutation(&mut lines, &order, 1);
        apply_permutation(&mut lines, &inv, 1);
        assert_eq!(lines, before);
    }
}
