//! 구분자 변환 — 데이터를 실제로 재작성해 구분자를 바꾼다.
//!
//! 툴바의 `Delimiter` 드롭다운(보기 설정)과 역할이 다르다. 저쪽은 바이트를
//! 어떻게 컬럼으로 나눌지를 정할 뿐이고, 이 모듈은 **바이트 자체**를 고친다.
//!
//! GUI 의존성 없음(이 코드베이스의 모듈 규율 — 순수 로직은 `app.rs` 밖에).

use crate::parse::{join_fields, split_fields};
use rayon::prelude::*;

/// 한 행의 구분자를 `old`에서 `new`로 바꾼 문자열.
///
/// # 왜 단순 바이트 치환이 아닌가
///
/// 인용부호가 두 방향 모두에서 규칙을 깬다.
///
/// **인용 안의 구분자를 건드리면 안 된다.** `홍길동,"서울, 강남구",30`은 3필드인데,
/// 콤마를 전부 탭으로 바꾸면 인용 **안**의 콤마까지 바뀌어 4필드로 깨진다.
///
/// **새 구분자가 값에 있으면 없던 인용을 만들어야 한다.** 탭 구분
/// `홍길동\t서울, 강남구`를 콤마로 바꾸면 `홍길동,"서울, 강남구"`가 되어야
/// 다시 읽을 때 2필드다. 바이트 치환은 인용부호를 만들어내지 못한다.
///
/// # 빠른 경로
///
/// 위 두 함정은 행에 `"`도 없고 `new` 바이트도 없으면 **둘 다 성립하지 않는다.**
/// 그때는 `old` 바이트를 `new`로 갈아 끼우는 것으로 충분하고, `split_fields`의
/// 필드별 String 할당(행당 필드 수만큼)을 통째로 건너뛴다. 실제 CSV/TSV는
/// 대다수 행이 이 경로를 탄다.
///
/// **조건이 완전한 근거.** `join_fields`가 인용을 씌우는 조건은 값에
/// `new` / `"` / `\n` / `\r` 중 하나라도 있을 때다. 앞의 둘은 위 조건이 직접
/// 배제하고, `\n`/`\r`는 **`EditBuffer.lines[i]`가 개행을 담지 않는다**는
/// 이 코드베이스의 불변식이 배제한다(줄바꿈은 저장할 때 붙는다). 따라서 빠른
/// 경로를 탄 행은 인용이 전혀 필요 없고, 결과가 폴백과 비트 단위로 같다.
///
/// 이 규율은 찾기의 바이트 판정과 같다 — **바이트 경로는 "확정" 아니면 "폴백"만
/// 결론지을 수 있고, 절대 "아님"을 결론짓지 않는다.** 애매하면 폴백이 정답이다.
/// 폴백은 느릴 뿐 틀리지 않는다.
pub fn convert_line(line: &str, old: u8, new: u8) -> String {
    let (out, _) = convert_line_traced(line, old, new);
    out
}

/// 어느 경로를 탔는지.
///
/// 테스트가 **실제로 실행된 경로**를 세기 위해 존재한다. 테스트가 판정식을
/// 복사해 자기편에서 다시 계산하면, 진짜 빠른 경로가 통째로 죽어 있어도
/// (항상 폴백) 커버리지 검사가 멀쩡히 통과한다 — 이 코드베이스에서 반복해서
/// 나온 결함이라 여기서는 구조적으로 막는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertPath {
    /// 바이트 교체만으로 끝났다.
    Fast,
    /// `split_fields` + `join_fields`로 파싱해서 다시 조립했다.
    Fallback,
}

/// `convert_line`의 본체. 결과와 **탄 경로**를 함께 돌려준다.
pub fn convert_line_traced(line: &str, old: u8, new: u8) -> (String, ConvertPath) {
    // 비ASCII 구분자는 바이트 교체가 UTF-8 멀티바이트 문자의 내부 바이트를
    // 건드릴 수 있다. 다이얼로그가 ASCII만 허용하지만 방어적으로 폴백한다.
    if !old.is_ascii() || !new.is_ascii() {
        return (join_fields(&split_fields(line, old), new), ConvertPath::Fallback);
    }
    // 빠른 경로: 인용부호도 새 구분자도 없다 → 인용이 필요한 값이 하나도 없다.
    if memchr::memchr2(b'"', new, line.as_bytes()).is_none() {
        // `old`를 `new`로 갈아 끼우기만 하면 된다. 둘 다 ASCII이므로 UTF-8
        // 후속 바이트(전부 0x80 이상)와 겹치지 않아 멀티바이트 문자를 깨지
        // 않는다. 길이도 변하지 않는다.
        let out = line
            .chars()
            .map(|c| if c == old as char { new as char } else { c })
            .collect();
        return (out, ConvertPath::Fast);
    }
    // 폴백: 인용부호가 있거나 값에 새 구분자가 있다 → 파싱해서 다시 조립한다.
    (join_fields(&split_fields(line, old), new), ConvertPath::Fallback)
}

/// 문서 전체 변환. 실제로 **달라진** 행만 `(행번호, 새 텍스트)`로 돌려준다.
///
/// 인덱스 오름차순이 보장된다 — rayon의 `collect`가 입력 순서를 보존한다
/// (`sort::sort_lines`, `find::replace_all`과 같은 패턴).
///
/// 달라지지 않은 행을 걸러내는 이유: 되돌리기 스택에 담기면 Ctrl+Z 한 번이
/// 아무것도 안 바꾸는 유령 단계가 되고 dirty가 거짓으로 선다. 변환은 인용
/// 정규화 때문에 "구분자는 바뀌었지만 이 행은 원래 필드가 하나뿐이라
/// 그대로"인 경우가 흔하다.
pub fn convert_all(lines: &[String], old: u8, new: u8) -> Vec<(usize, String)> {
    lines
        .par_iter()
        .enumerate()
        .filter_map(|(i, line)| {
            let converted = convert_line(line, old, new);
            if converted == *line {
                None
            } else {
                Some((i, converted))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 참조 구현 — 빠른 경로 없이 항상 파싱해서 다시 조립한다.
    /// `convert_line`의 오라클.
    fn reference(line: &str, old: u8, new: u8) -> String {
        join_fields(&split_fields(line, old), new)
    }


    #[test]
    fn quoted_delimiter_is_preserved() {
        // 인용 안의 콤마는 데이터다 — 탭으로 바뀌면 안 된다.
        let got = convert_line("홍길동,\"서울, 강남구\",30", b',', b'\t');
        assert_eq!(got, "홍길동\t서울, 강남구\t30");
        // 다시 읽으면 3필드여야 한다.
        assert_eq!(split_fields(&got, b'\t').len(), 3);
    }

    #[test]
    fn new_delimiter_in_value_gets_quoted() {
        // 탭 구분 2필드. 콤마로 바꾸면 값의 콤마 때문에 인용이 **생겨야** 한다.
        let got = convert_line("홍길동\t서울, 강남구", b'\t', b',');
        assert_eq!(got, "홍길동,\"서울, 강남구\"");
        assert_eq!(split_fields(&got, b',').len(), 2);
    }

    #[test]
    fn plain_row_converts() {
        assert_eq!(convert_line("a,b,c", b',', b'\t'), "a\tb\tc");
        assert_eq!(convert_line("a\tb\tc", b'\t', b'|'), "a|b|c");
        assert_eq!(convert_line("a|b|c", b'|', b'~'), "a~b~c");
    }

    #[test]
    fn empty_and_single_field() {
        assert_eq!(convert_line("", b',', b'\t'), "");
        assert_eq!(convert_line("solo", b',', b'\t'), "solo");
        assert_eq!(convert_line(",,", b',', b'\t'), "\t\t");
    }

    #[test]
    fn non_ascii_content_untouched() {
        // 한글은 UTF-8 멀티바이트다. 구분자 교체가 그 내부 바이트를 건드리면
        // 글자가 깨진다.
        let got = convert_line("한국어,인도네시아,日本語", b',', b'\t');
        assert_eq!(got, "한국어\t인도네시아\t日本語");
    }

    /// 전수 코퍼스 차등 테스트. 알파벳 `{a, ", ,, \t, |}` 길이 0~4 전수를
    /// 구분자 쌍마다 태워 `convert_line`이 참조 구현과 **정확히** 같은지 본다.
    ///
    /// 랜덤이 아니라 전수인 이유: 이 코드베이스에서 랜덤 79.7만 건이 놓친
    /// 따옴표 위음성을 전수 프로브가 잡은 적이 있다. 따옴표는 조합이 좁고
    /// 함정이 깊어 전수가 맞다.
    #[test]
    fn differential_exhaustive_corpus() {
        let alphabet = ['a', '"', ',', '\t', '|'];
        let delims = [b',', b'\t', b'|'];
        let mut fast = 0usize;
        let mut slow = 0usize;
        let mut checked = 0usize;

        for len in 0..=4u32 {
            // 길이 len의 모든 조합 = alphabet.len()^len 개. 각 조합을 정수
            // n의 base-|alphabet| 표현으로 만든다(자리올림 루프보다 틀릴 구석이
            // 없다).
            let total = (alphabet.len() as u64).pow(len);
            for n in 0..total {
                let mut rest = n;
                let mut s = String::new();
                for _ in 0..len {
                    s.push(alphabet[(rest % alphabet.len() as u64) as usize]);
                    rest /= alphabet.len() as u64;
                }
                for &old in &delims {
                    for &new in &delims {
                        if old == new {
                            continue;
                        }
                        let want = reference(&s, old, new);
                        // 경로를 **실행 결과로** 받는다. 테스트가 판정식을
                        // 다시 계산하면 빠른 경로가 죽어도 알 수 없다.
                        let (got, path) = convert_line_traced(&s, old, new);
                        assert_eq!(got, want, "input {s:?} old {old:?} new {new:?}");
                        match path {
                            ConvertPath::Fast => fast += 1,
                            ConvertPath::Fallback => slow += 1,
                        }
                        checked += 1;
                    }
                }
            }
        }

        assert!(checked > 1000, "코퍼스가 너무 작다: {checked}");
        // 두 경로가 **둘 다** 실제로 돌았는지. 이게 없으면 빠른 경로가 통째로
        // 죽어 있어도(항상 폴백) 위 assert들이 멀쩡히 통과한다.
        assert!(fast > 0, "빠른 경로를 탄 입력이 없다 — 커버리지 구멍");
        assert!(slow > 0, "폴백을 탄 입력이 없다 — 커버리지 구멍");
    }

    /// 왕복: `,` → `\t` → `,`. 문자열이 아니라 **필드 값**이 돌아와야 한다
    /// (인용이 늘어날 수 있으므로 문자열 동일성은 성립하지 않는다).
    #[test]
    fn roundtrip_preserves_field_values() {
        let cases = [
            "a,b,c",
            "홍길동,\"서울, 강남구\",30",
            "\"quoted\",plain",
            "a,,c",
            "\"has\"\"escaped\",x",
        ];
        for src in cases {
            let want: Vec<String> = split_fields(src, b',');
            let there = convert_line(src, b',', b'\t');
            let back = convert_line(&there, b'\t', b',');
            let got: Vec<String> = split_fields(&back, b',');
            assert_eq!(got, want, "왕복 실패: {src:?} → {there:?} → {back:?}");
        }
    }

    #[test]
    fn convert_all_reports_only_changed_rows() {
        let lines: Vec<String> = ["a,b", "solo", "c,d"].iter().map(|s| s.to_string()).collect();
        let changed = convert_all(&lines, b',', b'\t');
        // "solo"는 구분자가 없어 변환해도 그대로다 → 빠져야 한다.
        assert_eq!(changed, vec![(0, "a\tb".to_owned()), (2, "c\td".to_owned())]);
    }

    #[test]
    fn convert_all_preserves_index_order() {
        // 병렬 처리가 순서를 흐트러뜨리지 않는지. 행이 많아야 실제로 여러
        // 스레드로 갈린다.
        let lines: Vec<String> = (0..5000).map(|i| format!("r{i},x,y")).collect();
        let changed = convert_all(&lines, b',', b'\t');
        assert_eq!(changed.len(), 5000);
        for (n, (i, _)) in changed.iter().enumerate() {
            assert_eq!(*i, n, "인덱스가 오름차순이 아니다");
        }
    }

    #[test]
    fn convert_all_empty_when_nothing_changes() {
        let lines: Vec<String> = ["solo", "alone"].iter().map(|s| s.to_string()).collect();
        assert!(convert_all(&lines, b',', b'\t').is_empty());
    }
}
