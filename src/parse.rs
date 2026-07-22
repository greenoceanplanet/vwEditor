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
        let slice = &head[..head.len() - cut];
        if !slice.is_empty() && std::str::from_utf8(slice).is_ok() {
            return Encoding::Utf8;
        }
    }
    Encoding::Cp949
}

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
