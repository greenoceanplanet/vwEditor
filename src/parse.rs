use encoding_rs::{EUC_KR, UTF_16BE, UTF_16LE, UTF_8};

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

/// 화면을 어떻게 나눌지 결정하는 구분 모드.
/// - `None`: 구분하지 않고 한 줄 전체를 텍스트로 표시(일반 텍스트/JSON/로그).
/// - `Char(b)`: 바이트 `b`(콤마/탭/파이프/세미콜론/커스텀 한 글자)로 필드 분리.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorMode {
    None,
    Char(u8),
}

const DELIMITER_CANDIDATES: [u8; 4] = [b',', b'\t', b'|', b';'];

/// 확장자 우선, 애매하면 앞부분 여러 줄에서 가장 일관된 구분자 선택.
/// 어떤 후보도 줄당 평균 1회 미만이면 구분자가 없다고 보고 `SeparatorMode::None`
/// (텍스트 모드)로 연다 — 일반 텍스트/JSON/로그가 자연히 텍스트 뷰로 열린다.
pub fn detect_separator(path: &std::path::Path, head_lines: &[&str]) -> SeparatorMode {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "tsv" | "tab" => return SeparatorMode::Char(b'\t'),
            "csv" => return SeparatorMode::Char(b','),
            "psv" => return SeparatorMode::Char(b'|'),
            // 명백한 비표형 확장자는 바로 텍스트 모드.
            "json" | "log" | "md" | "txt" => {
                // txt/md/log는 내용에 실제로 구분자가 일관되게 있으면 표로,
                // 아니면 텍스트로. json은 거의 항상 텍스트지만 아래 내용 분석에
                // 맡겨 일관된 구분자가 있으면 표로 열 수 있게 둔다.
            }
            _ => {}
        }
    }
    // 내용 분석: 각 후보의 줄당 등장 횟수 분산이 가장 낮고 평균이 1 이상인 것.
    let mut best: Option<u8> = None;
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
            best = Some(cand);
        }
    }
    match best {
        Some(b) => SeparatorMode::Char(b),
        None => SeparatorMode::None, // 일관된 구분자 없음 → 텍스트 모드
    }
}

/// 한 줄을 구분자로 분리. 따옴표(")로 감싼 필드 안의 구분자는 무시.
///
/// 구현 노트(csv-core 0.1.13 API 확인 결과, 브리프의 의사코드에서 편차):
/// `read_record` 는 입력을 다 소비해도 개행이 없으면 마지막 필드의 끝 위치를
/// `ends` 에 즉시 쓰지 않는다(csv-core 소스 주석: "they'll be missing the
/// final end position of the final field in `ends`"). 이를 채우려면 빈
/// 입력 슬라이스로 한 번 더 호출해 EOF 를 알려야 하는데, 그 호출은
/// `output` 버퍼에는 아무것도 새로 쓰지 않고(nout=0) `ends` 에만 이전
/// 호출까지 누적된 위치(`output_pos`)를 추가한다. 즉 `out`/`ends` 를
/// 여러 번의 `read_record` 호출에 걸쳐 "이어붙여" 읽으면 좌표가 어긋난다.
/// 따라서 출력 버퍼 크기를 입력보다 항상 크게 잡아 실질적으로 데이터 쓰기는
/// 단 한 번의 호출로 끝나도록 하고, 그 뒤에는 EOF 플러시 호출(nend 증가만)
/// 만 반복해서 받도록 한다. 필드 벡터는 매 호출 후가 아니라 루프 종료 후
/// 최종 `ends` 스냅샷을 기준으로 한 번에 구성한다.
pub fn split_fields(line: &str, delim: u8) -> Vec<String> {
    let mut reader = csv_core::ReaderBuilder::new()
        .delimiter(delim)
        .build();
    let bytes = line.as_bytes();
    // 넉넉한 출력 버퍼: 필드 구분자/따옴표가 제거되므로 출력은 입력보다 클 수 없다.
    let mut out = vec![0u8; bytes.len() + 1];
    let mut ends = vec![0usize; bytes.len() + 1];
    let mut input = bytes;
    let mut total_nend = 0usize;
    loop {
        let (result, nin, _nout, nend) =
            reader.read_record(input, &mut out, &mut ends[total_nend..]);
        total_nend += nend;
        input = &input[nin..];
        match result {
            csv_core::ReadRecordResult::End => break,
            csv_core::ReadRecordResult::InputEmpty => {
                if input.is_empty() {
                    // 다음 호출에서 빈 슬라이스로 EOF 를 알려 마지막 필드를 flush.
                    continue;
                }
            }
            csv_core::ReadRecordResult::OutputFull => {
                out.resize(out.len() * 2, 0);
            }
            csv_core::ReadRecordResult::OutputEndsFull => {
                ends.resize(ends.len() * 2, 0);
            }
            csv_core::ReadRecordResult::Record => {
                break; // 한 줄짜리 입력이므로 레코드 하나면 끝.
            }
        }
        if input.is_empty() && total_nend > 0 {
            // EOF flush 까지 받아 total_nend 가 이미 채워졌다면 종료.
            break;
        }
    }
    let mut fields = Vec::new();
    for i in 0..total_nend {
        let start = if i == 0 { 0 } else { ends[i - 1] };
        let end = ends[i];
        fields.push(String::from_utf8_lossy(&out[start..end]).into_owned());
    }
    if fields.is_empty() {
        fields.push(String::new());
    }
    fields
}

/// 한 줄의 raw 바이트에서 `col`번째 필드의 byte 범위 `[start, end)`를 찾아 그
/// 슬라이스를 반환한다. **String/Vec 할당이 전혀 없다** — 정렬 키 추출처럼
/// 수억 행을 도는 hot path 전용. 해당 컬럼이 없으면 None.
///
/// 규칙(split_fields와 동일 정신, 단일바이트 구분자 전용):
/// - `delim`(콤마/탭/파이프/세미콜론/커스텀 한 글자)로 필드를 나눈다.
/// - 큰따옴표(`"`)로 감싼 필드 안의 구분자는 무시한다(따옴표는 결과 슬라이스에
///   그대로 포함 — 정렬 비교에는 무해하고, 할당 없이 슬라이스를 돌려주기 위함).
/// - 개행 문자는 호출측이 미리 제거(trim)했다고 가정한다.
///
/// 주의: 이 함수는 **바이트 단위**로 동작하므로 구분자가 단일 ASCII 바이트인
/// 인코딩(UTF-8/CP949)에서 정확하다. UTF-16은 구분자가 2바이트라 호출측이
/// 별도 경로(String 기반)로 처리해야 한다.
pub fn field_slice(line: &[u8], delim: u8, col: usize) -> Option<&[u8]> {
    let mut idx = 0usize; // 현재 몇 번째 필드인지
    let mut field_start = 0usize;
    let mut in_quotes = false;
    let mut i = 0usize;
    while i < line.len() {
        let b = line[i];
        if b == b'"' {
            // 따옴표 토글. (RFC 4180의 "" 이스케이프는 정렬 비교에 영향이
            // 미미하므로 단순 토글로 둔다.)
            in_quotes = !in_quotes;
        } else if b == delim && !in_quotes {
            if idx == col {
                return Some(&line[field_start..i]);
            }
            idx += 1;
            field_start = i + 1;
        }
        i += 1;
    }
    // 마지막 필드(구분자 뒤 끝까지).
    if idx == col {
        return Some(&line[field_start..]);
    }
    None
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    #[test]
    fn tsv_extension_picks_tab() {
        assert_eq!(
            detect_separator(Path::new("a.tsv"), &["x\ty"]),
            SeparatorMode::Char(b'\t')
        );
    }

    #[test]
    fn csv_extension_picks_comma() {
        assert_eq!(
            detect_separator(Path::new("a.csv"), &["x,y"]),
            SeparatorMode::Char(b',')
        );
    }

    #[test]
    fn txt_content_picks_consistent_delimiter() {
        // 매 줄 파이프가 정확히 2개로 일관 → 파이프
        let lines = ["a|b|c", "d|e|f", "g|h|i"];
        assert_eq!(
            detect_separator(Path::new("a.txt"), &lines),
            SeparatorMode::Char(b'|')
        );
    }

    #[test]
    fn plain_text_without_delimiter_is_none() {
        // 구분자가 거의 없는 일반 텍스트 → 텍스트 모드(None)
        let lines = [
            "This is just a sentence.",
            "Another line of prose here",
            "No delimiters to speak of",
        ];
        assert_eq!(detect_separator(Path::new("notes.txt"), &lines), SeparatorMode::None);
    }

    #[test]
    fn json_content_is_none() {
        // JSON은 줄당 일관된 표 구분자가 없음 → 텍스트 모드
        let lines = [
            "{",
            "  \"name\": \"Alice\",",
            "  \"age\": 30",
            "}",
        ];
        assert_eq!(detect_separator(Path::new("data.json"), &lines), SeparatorMode::None);
    }

    #[test]
    fn tsv_extension_wins_even_if_content_sparse() {
        // 확장자가 tsv면 내용이 애매해도 탭으로(확장자 우선).
        assert_eq!(
            detect_separator(Path::new("a.tsv"), &["single_column_line"]),
            SeparatorMode::Char(b'\t')
        );
    }

    #[test]
    fn split_basic_fields() {
        assert_eq!(split_fields("a,b,c", b','), vec!["a", "b", "c"]);
    }

    #[test]
    fn field_slice_basic() {
        assert_eq!(field_slice(b"a,b,c", b',', 0), Some(&b"a"[..]));
        assert_eq!(field_slice(b"a,b,c", b',', 1), Some(&b"b"[..]));
        assert_eq!(field_slice(b"a,b,c", b',', 2), Some(&b"c"[..]));
        assert_eq!(field_slice(b"a,b,c", b',', 3), None);
    }

    #[test]
    fn field_slice_empty_fields() {
        assert_eq!(field_slice(b"a,,c", b',', 1), Some(&b""[..]));
        assert_eq!(field_slice(b",x", b',', 0), Some(&b""[..]));
    }

    #[test]
    fn field_slice_quoted_delimiter_ignored() {
        // "a,b" 안의 콤마는 무시되어 한 필드. col 0 = 따옴표 포함 슬라이스, col 1 = c
        assert_eq!(field_slice(b"\"a,b\",c", b',', 0), Some(&b"\"a,b\""[..]));
        assert_eq!(field_slice(b"\"a,b\",c", b',', 1), Some(&b"c"[..]));
    }

    #[test]
    fn field_slice_tab_delim() {
        assert_eq!(field_slice(b"x\ty\tz", b'\t', 2), Some(&b"z"[..]));
    }

    #[test]
    fn field_slice_last_field_to_end() {
        assert_eq!(field_slice(b"a,bcd", b',', 1), Some(&b"bcd"[..]));
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
}
