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
