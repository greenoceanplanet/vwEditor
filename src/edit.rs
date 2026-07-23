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
}

/// mmap 바이트를 개행(`\n`)으로 분할하고 각 줄을 `enc`로 디코딩해 EditBuffer를 만든다.
/// - `\r\n`이 하나라도 있으면 newline=CrLf, 아니면 Lf.
/// - 각 줄에서 뒤쪽 `\r`/`\n`을 제거한다.
/// - 마지막 바이트가 개행이면 그 뒤에 빈 줄을 추가하지 않는다(파일 끝 개행은 종결자).
/// - 빈 파일이면 `lines = [""]`(빈 한 줄).
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
}
