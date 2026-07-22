use crate::parse::Encoding;

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
