//! 찾기/바꾸기 순수 로직. `edit.rs`/`save.rs`와 같은 규율로 egui를 전혀
//! 알지 못한다 — UI(`app.rs`)는 여기 함수를 부르는 얇은 껍데기다.
//!
//! **모든 위치는 char 인덱스다.** 이 코드베이스의 `TextPos.col`이 char
//! 인덱스이므로(`edit.rs:74`) 바이트 인덱스를 섞으면 한글/이모지에서 커서가
//! 엉뚱한 곳으로 가고 `String` 슬라이싱이 패닉한다.

use crate::edit::TextPos;

/// 찾기 옵션.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindOptions {
    /// 대소문자 구분.
    pub match_case: bool,
    /// 단어 단위로만 일치.
    pub whole_word: bool,
}

/// 한 행 안에서 찾은 위치. col은 **문자(char) 인덱스** — 이 코드베이스의
/// TextPos.col과 같은 단위다(바이트가 아니다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub col: usize,
    /// 일치한 길이(문자 수).
    pub len: usize,
}

/// 단어 문자 판정. 한글도 `is_alphanumeric()`이 참이므로 `"가나다"`에서
/// `"나"`를 단어 단위로 찾으면 매치가 없다 — 의도된 동작이다(브리프 확정).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 대소문자 무시 비교를 위해 문자열을 "소문자 char 열 + 원본 char 인덱스 역맵"
/// 으로 펼친다.
///
/// **왜 `to_lowercase()` 결과에서 찾은 인덱스를 그대로 쓰면 안 되는가.**
/// `char::to_lowercase()`는 1:N이다(터키어 `İ` → `i` + U+0307 두 char).
/// 소문자화한 문자열에서 얻은 인덱스를 원본에 그대로 쓰면 그 지점 이후로
/// 전부 어긋나고, 원본이 한글/이모지면 어긋난 인덱스가 곧 잘못된 커서/
/// 잘못된 치환 범위가 된다. 그래서 소문자 char 하나마다 **그것이 어느 원본
/// char에서 나왔는지**를 같은 길이의 배열로 함께 들고 다닌다. 매치를 찾으면
/// 역맵으로 원본 char 인덱스를 복원하므로 확장이 몇 개로 늘어나든 안전하다.
///
/// `match_case`가 참이면 접힘 없이 원본 char를 그대로 담는다(역맵은 항등).
fn folded(s: &str, match_case: bool) -> (Vec<char>, Vec<usize>) {
    let mut chars = Vec::new();
    let mut origin = Vec::new();
    for (i, c) in s.chars().enumerate() {
        if match_case {
            chars.push(c);
            origin.push(i);
        } else {
            for lc in c.to_lowercase() {
                chars.push(lc);
                origin.push(i);
            }
        }
    }
    (chars, origin)
}

/// 한 행(hay) 안에서 needle이 나오는 모든 위치를 문자 인덱스로 반환.
/// needle이 비면 빈 벡터(무한 루프 방지).
///
/// 반환값의 `len`은 **원본 hay에서 소비한 char 수**다 — 대소문자 무시로
/// 접힘 길이가 달라져도 호출부가 원본을 그대로 잘라낼 수 있어야 하므로
/// 접힘 길이가 아니라 원본 길이를 돌려준다.
pub fn find_in_line(hay: &str, needle: &str, opts: &FindOptions) -> Vec<(usize, usize)> {
    // 빈 needle은 모든 위치에서 매치가 되어 호출부를 무한 루프에 빠뜨린다.
    if needle.is_empty() {
        return Vec::new();
    }
    let (h, origin) = folded(hay, opts.match_case);
    let (n, _) = folded(needle, opts.match_case);
    if n.is_empty() || h.len() < n.len() {
        return Vec::new();
    }
    // 단어 경계 판정은 **원본** char 기준으로 한다(접힘 char가 아니라).
    let hay_chars: Vec<char> = hay.chars().collect();
    let hay_len = hay_chars.len();

    let mut out = Vec::new();
    let mut i = 0usize;
    while i + n.len() <= h.len() {
        if h[i..i + n.len()] != n[..] {
            i += 1;
            continue;
        }
        // 접힘 인덱스 → 원본 char 인덱스. 끝은 "매치 뒤 첫 접힘 char의 원본
        // 인덱스"인데, 매치가 hay 끝에 닿으면 그 char가 없으므로 hay 길이.
        let start = origin[i];
        let end = origin.get(i + n.len()).copied().unwrap_or(hay_len);
        // 한 원본 char가 여러 접힘 char로 펼쳐진 경우, 매치가 그 char의
        // **중간**에서 시작/끝날 수 있다(예: `İ` → `i` + U+0307에서 U+0307만
        // 걸치는 매치). 그런 매치는 원본 char를 반으로 쪼개야 표현되므로
        // 인정하지 않는다 — 인정하면 치환이 원본 char를 깨뜨린다.
        let starts_on_boundary = i == 0 || origin[i - 1] != start;
        let ends_on_boundary = i + n.len() >= origin.len() || origin[i + n.len() - 1] != end;
        if !starts_on_boundary || !ends_on_boundary || end <= start {
            i += 1;
            continue;
        }
        if opts.whole_word {
            let before_ok = start == 0 || !is_word_char(hay_chars[start - 1]);
            let after_ok = end >= hay_len || !is_word_char(hay_chars[end]);
            if !(before_ok && after_ok) {
                i += 1;
                continue;
            }
        }
        out.push((start, end - start));
        // 겹치지 않는 매치: 찾은 뒤 needle 길이만큼 건너뛴다.
        // ("aaa"에서 "aa"는 위치 0 하나뿐이다.)
        i += n.len();
    }
    out
}

/// 치환문에 든 `\n`/`\r`를 공백으로 바꾼다. `lines[i]`에 개행이 박히면
/// "한 줄 = 한 행" 불변식이 깨져 표 모드 행번호와 저장 결과가 어긋난다
/// (`edit.rs`의 `sanitize_cell_value`가 같은 이유로 존재한다).
///
/// **왜 `edit.rs`의 것을 `pub(crate)`로 올려 쓰지 않았는가.** 그쪽은 "셀 값"
/// 계약이라 CSV 인용/필드 분리 규칙과 함께 움직이는 반면 여기는 "줄 안의 임의
/// 텍스트" 계약이다. 두 곳의 요구가 나중에 갈릴 수 있는데(예: 셀만 탭도
/// 막고 싶어지는 경우) 지금 묶어 두면 한쪽 요구가 다른 쪽을 조용히 끌고
/// 간다. 지킬 불변식이 같을 뿐 계약이 다르므로 3줄을 각자 갖는다.
/// (`app.rs`의 "한 곳만 바꾸기"도 같은 규칙을 써야 하므로 `pub(crate)`다 —
///  그쪽은 `replace_in_line`을 거치지 않고 매치 구간만 직접 갈아 끼운다.)
pub(crate) fn sanitize_for_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// 한 행 안에서 needle을 replacement로 모두 바꾼 새 문자열과 바뀐 횟수.
pub fn replace_in_line(
    hay: &str,
    needle: &str,
    replacement: &str,
    opts: &FindOptions,
) -> (String, usize) {
    if needle.is_empty() {
        return (hay.to_owned(), 0);
    }
    let hits = find_in_line(hay, needle, opts);
    if hits.is_empty() {
        return (hay.to_owned(), 0);
    }
    let rep = sanitize_for_line(replacement);
    // 매치 위치는 char 인덱스이므로 바이트 오프셋으로 옮겨 잘라 붙인다.
    // `char_indices`를 한 번만 돌아 (char 인덱스 → 바이트 오프셋) 표를 만든다.
    let mut byte_of: Vec<usize> = hay.char_indices().map(|(b, _)| b).collect();
    byte_of.push(hay.len()); // 끝 경계
    let mut out = String::with_capacity(hay.len());
    let mut cursor = 0usize; // 아직 복사하지 않은 원본 바이트 시작
    // find_in_line은 앞에서 뒤로 정렬된 비중첩 매치를 준다. **앞에서부터**
    // 새 문자열을 조립하면 치환문 길이가 원문과 달라도 인덱스가 어긋날 여지가
    // 없다(제자리 수정이 아니라 복사이므로). in-place `replace_range`를 쓴다면
    // 반드시 뒤에서 앞으로 가야 하지만, 여기서는 그 함정 자체를 없앤다.
    for (col, len) in &hits {
        let s = byte_of[*col];
        let e = byte_of[col + len];
        out.push_str(&hay[cursor..s]);
        out.push_str(&rep);
        cursor = e;
    }
    out.push_str(&hay[cursor..]);
    (out, hits.len())
}

/// `from` 위치 **다음**의 첫 매치를 찾는다. 문서 끝에 닿으면 처음부터
/// 다시 돌아 `from`까지 훑는다(wrap around). 없으면 None.
///
/// `line_count`: 전체 논리 행 수.
/// `get_line`: 논리 행 → 텍스트. None이면 그 행은 건너뛴다(인덱싱이 아직
///             안 끝난 뷰 모드에서 일어날 수 있다).
///
/// `from`은 **포함하지 않는다** — 같은 자리를 다시 돌려주면 "다음 찾기"가
/// 제자리걸음한다. 시작 행에서는 `from.col`보다 **큰** col만 본다.
/// 매치가 하나뿐이고 그게 `from` 자리면 한 바퀴 돌아 자기 자신을 준다
/// (에디터 관례).
pub fn find_next(
    line_count: usize,
    from: TextPos,
    needle: &str,
    opts: &FindOptions,
    get_line: impl Fn(usize) -> Option<String>,
) -> Option<Match> {
    if line_count == 0 || needle.is_empty() {
        return None;
    }
    let start = from.line.min(line_count - 1);
    // 시작 행을 두 번(앞부분 남은 구간 + wrap 뒤 앞 구간) 보므로 line_count + 1
    // 번만 돈다 — 그래야 "정확히 한 바퀴"가 되고 무한 루프가 생기지 않는다.
    for step in 0..=line_count {
        let line = (start + step) % line_count;
        let Some(text) = get_line(line) else { continue };
        for (col, len) in find_in_line(&text, needle, opts) {
            // 첫 바퀴의 시작 행에서는 from보다 뒤만, 마지막(wrap 후 되돌아온)
            // 시작 행에서는 from 자리까지 포함해 본다 — 매치가 하나뿐일 때
            // 자기 자신으로 돌아오게 하는 것이 이 포함 처리다.
            if step == 0 && line == start && col <= from.col {
                continue;
            }
            if step == line_count && col > from.col {
                break;
            }
            return Some(Match { line, col, len });
        }
    }
    None
}

/// `from` 위치 **이전**의 마지막 매치를 찾는다. 마찬가지로 wrap around.
pub fn find_prev(
    line_count: usize,
    from: TextPos,
    needle: &str,
    opts: &FindOptions,
    get_line: impl Fn(usize) -> Option<String>,
) -> Option<Match> {
    if line_count == 0 || needle.is_empty() {
        return None;
    }
    let start = from.line.min(line_count - 1);
    for step in 0..=line_count {
        // 뒤로 도는 인덱스. usize 언더플로를 피하려고 line_count를 더해 돈다.
        let line = (start + line_count - step % line_count) % line_count;
        let Some(text) = get_line(line) else { continue };
        for (col, len) in find_in_line(&text, needle, opts).into_iter().rev() {
            if step == 0 && line == start && col >= from.col {
                continue;
            }
            if step == line_count && col < from.col {
                break;
            }
            return Some(Match { line, col, len });
        }
    }
    None
}

/// 모든 행에서 needle을 replacement로 바꾼다. 바뀐 행만 (행번호, 새 텍스트)로
/// 반환한다 — 호출부가 그것만 버퍼에 반영하고 undo에 기록할 수 있게.
/// 총 치환 횟수도 함께 반환.
///
/// 편집 모드 전용이다(뷰 모드는 버퍼가 없어 바꿀 수 없다). 그래서 `&[String]`을
/// 직접 받는다. 바뀌지 않은 행을 결과에 넣지 않는 것이 핵심이다 — 200만 행에서
/// 10행만 바뀌었는데 200만 개를 돌려주면 undo 스택이 파일 전체를 복제한다.
pub fn replace_all(
    lines: &[String],
    needle: &str,
    replacement: &str,
    opts: &FindOptions,
) -> (Vec<(usize, String)>, usize) {
    if needle.is_empty() {
        return (Vec::new(), 0);
    }
    let mut changed = Vec::new();
    let mut total = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let (new, n) = replace_in_line(line, needle, replacement, opts);
        if n > 0 {
            total += n;
            changed.push((i, new));
        }
    }
    (changed, total)
}

/// 검색어가 들어 있는 논리 행들의 행번호를 **훑은 순서 그대로** 모은다.
/// needle이 비면 빈 결과(`find_in_line`과 같은 규칙 — 빈 검색어는 매치가 없다).
///
/// `get_line`이 None을 주는 행은 건너뛴다(뷰 모드에서 인덱싱이 아직 그 행에
/// 닿지 않은 경우 — `find_next`가 같은 이유로 같은 처리를 한다).
///
/// **판정은 반드시 `find_in_line`으로 한다.** 여기서 `contains()` 같은 자체
/// 판정을 쓰면 대소문자/단어 단위 옵션이 찾기와 추출에서 서로 다르게 먹어,
/// "찾기로는 안 잡히는 행이 추출본에는 들어 있다"는 어긋남이 생긴다.
/// 매치 개수는 세지 않는다 — 한 행에 여러 번 나와도 그 행은 결과에 한 번뿐이다
/// (행 단위 추출이므로 개수는 의미가 없다).
///
/// `line_count`가 곧 훑는 범위다. 헤더 행을 제외하고 훑는 것은 호출부의
/// 몫이 아니라 `extract_plan`(`app.rs`)의 몫이다 — 여기는 "어느 구간을
/// 훑을 것인가"를 모른 채 "주어진 구간에서 매치 행을 고른다"만 한다.
pub fn matching_lines(
    line_count: usize,
    needle: &str,
    opts: &FindOptions,
    get_line: impl Fn(usize) -> Option<String>,
) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..line_count {
        let Some(text) = get_line(i) else { continue };
        if !find_in_line(&text, needle, opts).is_empty() {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(match_case: bool, whole_word: bool) -> FindOptions {
        FindOptions { match_case, whole_word }
    }

    fn lines(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    /// `get_line` 클로저의 표준 형태(테스트용).
    fn getter(v: Vec<String>) -> impl Fn(usize) -> Option<String> {
        move |i| v.get(i).cloned()
    }

    #[test]
    fn find_in_line_basic() {
        let hits = find_in_line("abc abc x", "abc", &opts(true, false));
        assert_eq!(hits, vec![(0, 3), (4, 3)]);
    }

    #[test]
    fn find_in_line_empty_needle_returns_nothing() {
        // 빈 needle이 매치를 내면 호출부가 무한 루프에 빠진다.
        assert!(find_in_line("abc", "", &opts(true, false)).is_empty());
        assert!(find_in_line("", "", &opts(false, false)).is_empty());
    }

    #[test]
    fn find_in_line_non_overlapping() {
        // "aaaa"에서 "aa"는 겹치지 않게 2개(0, 2). 3개(0,1,2)가 아니다.
        let hits = find_in_line("aaaa", "aa", &opts(true, false));
        assert_eq!(hits, vec![(0, 2), (2, 2)]);
        // "aaa"에서는 1개뿐.
        assert_eq!(find_in_line("aaa", "aa", &opts(true, false)), vec![(0, 2)]);
    }

    /// 바이트/문자 혼동을 잡는 핵심 테스트. `"가나다ABC가나"`의 char 열은
    /// 가(0) 나(1) 다(2) A(3) B(4) C(5) 가(6) 나(7) 이므로 두 번째 "가나"는
    /// **char 6**이다. 바이트 인덱스로 세면 한글 3바이트 × 3 + ASCII 3 = 12가
    /// 나온다 — 기대값이 6이 아니라 12/9 등이 나오면 바이트를 세고 있는 것이다.
    #[test]
    fn find_in_line_char_index_with_hangul() {
        let hits = find_in_line("가나다ABC가나", "가나", &opts(true, false));
        assert_eq!(hits, vec![(0, 2), (6, 2)]);
        // 돌려준 col/len으로 원본을 char 단위로 자르면 정확히 needle이어야 한다.
        let chars: Vec<char> = "가나다ABC가나".chars().collect();
        for (col, len) in hits {
            let got: String = chars[col..col + len].iter().collect();
            assert_eq!(got, "가나");
        }
    }

    #[test]
    fn find_in_line_case_insensitive() {
        let hits = find_in_line("Hello HELLO hello", "hello", &opts(false, false));
        assert_eq!(hits.len(), 3);
        assert_eq!(hits, vec![(0, 5), (6, 5), (12, 5)]);
    }

    #[test]
    fn find_in_line_case_insensitive_index_with_multibyte() {
        // 대소문자 무시 + 한글 혼합에서도 col이 원본 char 인덱스여야 한다.
        let hits = find_in_line("가나ABC가나abc", "ABC", &opts(false, false));
        assert_eq!(hits, vec![(2, 3), (7, 3)]);
    }

    #[test]
    fn find_in_line_case_insensitive_index_with_emoji() {
        // 이모지는 char 하나지만 바이트로는 4바이트다. 그 뒤 매치의 col이
        // 밀리면 바이트/문자 혼동이다.
        let hits = find_in_line("😀ABC😀abc", "abc", &opts(false, false));
        assert_eq!(hits, vec![(1, 3), (5, 3)]);
    }

    /// `to_lowercase()`가 1:N으로 늘어나는 char(터키어 `İ` → `i` + U+0307)가
    /// 앞에 있어도 그 뒤 매치의 col이 어긋나지 않아야 한다. 소문자 문자열에서
    /// 찾은 인덱스를 원본에 그대로 쓰면 여기서 한 칸씩 밀린다.
    #[test]
    fn find_in_line_case_insensitive_with_expanding_lowercase() {
        let hay = "İabc"; // char 4개: 'İ','a','b','c'
        let hits = find_in_line(hay, "ABC", &opts(false, false));
        assert_eq!(hits, vec![(1, 3)]);
        // 원본을 그 col/len으로 잘라내면 정확히 "abc"여야 한다.
        let chars: Vec<char> = hay.chars().collect();
        let got: String = chars[1..4].iter().collect();
        assert_eq!(got, "abc");
    }

    /// 접힘으로 펼쳐진 char의 **일부**만 걸치는 매치는 인정하지 않는다.
    /// (`İ`의 소문자 확장 뒤쪽 U+0307만 매치되면 원본 char를 반으로 쪼개야
    ///  하므로 치환이 불가능하다.)
    #[test]
    fn find_in_line_rejects_partial_expansion_match() {
        let hits = find_in_line("İ", "\u{0307}", &opts(false, false));
        assert!(hits.is_empty(), "확장 조각만 걸치는 매치는 버린다");
    }

    #[test]
    fn whole_word_rejects_substring() {
        assert!(find_in_line("testing", "test", &opts(true, true)).is_empty());
        assert_eq!(find_in_line("a test here", "test", &opts(true, true)), vec![(2, 4)]);
        // 문자열 시작/끝은 경계로 친다.
        assert_eq!(find_in_line("test", "test", &opts(true, true)), vec![(0, 4)]);
    }

    #[test]
    fn whole_word_underscore_is_word_char() {
        // `_`는 단어 문자 — "a_test"의 "test"는 단어 단위 매치가 아니다.
        assert!(find_in_line("a_test", "test", &opts(true, true)).is_empty());
    }

    #[test]
    fn whole_word_hangul_is_alphanumeric() {
        // 한글은 is_alphanumeric()이 true이므로 "가나다"에서 "나"는 매치 없음.
        assert!(find_in_line("가나다", "나", &opts(true, true)).is_empty());
        // 앞뒤가 단어 문자가 아니면 매치.
        assert_eq!(find_in_line("가 나 다", "나", &opts(true, true)), vec![(2, 1)]);
    }

    #[test]
    fn replace_in_line_counts_and_result() {
        let (s, n) = replace_in_line("a b a b", "a", "X", &opts(true, false));
        assert_eq!(s, "X b X b");
        assert_eq!(n, 2);
    }

    #[test]
    fn replace_in_line_no_match_is_unchanged() {
        let (s, n) = replace_in_line("abc", "zzz", "X", &opts(true, false));
        assert_eq!(s, "abc");
        assert_eq!(n, 0);
    }

    #[test]
    fn replace_in_line_empty_needle_is_noop() {
        let (s, n) = replace_in_line("abc", "", "X", &opts(true, false));
        assert_eq!(s, "abc");
        assert_eq!(n, 0);
    }

    #[test]
    fn replace_in_line_sanitizes_newline_in_replacement() {
        // lines[i] 불변식 회귀 테스트 — 치환문의 개행은 공백이 된다.
        let (s, n) = replace_in_line("a,b", "b", "x\ny", &opts(true, false));
        assert_eq!(s, "a,x y");
        assert_eq!(n, 1);
        assert!(!s.contains('\n'));
        let (s2, _) = replace_in_line("a", "a", "p\r\nq", &opts(true, false));
        assert_eq!(s2, "p  q", "\\r\\n 두 문자가 각각 공백으로");
        assert!(!s2.contains('\r'));
    }

    #[test]
    fn replace_in_line_longer_replacement() {
        // 치환문이 원문보다 길어도 뒤 매치의 인덱스가 어긋나면 안 된다.
        let (s, n) = replace_in_line("a-a-a", "a", "LONG", &opts(true, false));
        assert_eq!(s, "LONG-LONG-LONG");
        assert_eq!(n, 3);
    }

    #[test]
    fn replace_in_line_multibyte_slicing_is_safe() {
        // char 인덱스를 바이트로 옮기지 않으면 여기서 패닉하거나 깨진다.
        let (s, n) = replace_in_line("가나ABC가나", "ABC", "다", &opts(false, false));
        assert_eq!(s, "가나다가나");
        assert_eq!(n, 1);
    }

    #[test]
    fn replace_in_line_case_insensitive_keeps_surrounding_text() {
        let (s, n) = replace_in_line("Hello HELLO", "hello", "hi", &opts(false, false));
        assert_eq!(s, "hi hi");
        assert_eq!(n, 2);
    }

    #[test]
    fn find_next_moves_forward_and_wraps() {
        let v = lines(&["x a", "b a", "a c"]);
        let n = 3;
        // 0행 col2 매치 다음 → 1행 col2.
        let m = find_next(n, TextPos { line: 0, col: 2 }, "a", &opts(true, false), getter(v.clone()));
        assert_eq!(m, Some(Match { line: 1, col: 2, len: 1 }));
        // 마지막 매치(2행 col0) 뒤에서 찾으면 처음 매치(0행 col2)로 감싼다.
        let m = find_next(n, TextPos { line: 2, col: 0 }, "a", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_next_does_not_return_from_position() {
        // from이 매치 자리(0,0)면 그 다음 매치(0,2)를 준다.
        let v = lines(&["a a a"]);
        let m = find_next(1, TextPos { line: 0, col: 0 }, "a", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_next_single_match_wraps_to_itself() {
        // 매치가 하나뿐이고 그게 from 자리면 한 바퀴 돌아 자기 자신(에디터 관례).
        let v = lines(&["zzz", "hit", "zzz"]);
        let m = find_next(3, TextPos { line: 1, col: 0 }, "hit", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 1, col: 0, len: 3 }));
    }

    #[test]
    fn find_next_returns_none_when_absent() {
        let v = lines(&["a", "b", "c"]);
        let m = find_next(3, TextPos { line: 0, col: 0 }, "zzz", &opts(true, false), getter(v));
        assert_eq!(m, None);
    }

    #[test]
    fn find_next_skips_lines_that_return_none() {
        // 뷰 모드에서 인덱싱이 안 끝난 행은 get_line이 None을 준다.
        let m = find_next(4, TextPos { line: 0, col: 0 }, "hit", &opts(true, false), |i| {
            match i {
                0 => Some("nothing".to_string()),
                1 => None,
                2 => None,
                3 => Some("a hit".to_string()),
                _ => None,
            }
        });
        assert_eq!(m, Some(Match { line: 3, col: 2, len: 3 }));
    }

    #[test]
    fn find_next_empty_document_is_none() {
        assert_eq!(
            find_next(0, TextPos { line: 0, col: 0 }, "a", &opts(true, false), |_| None),
            None
        );
    }

    #[test]
    fn find_next_empty_needle_is_none() {
        let v = lines(&["abc"]);
        assert_eq!(
            find_next(1, TextPos { line: 0, col: 0 }, "", &opts(true, false), getter(v)),
            None
        );
    }

    #[test]
    fn find_next_from_beyond_last_line_is_clamped() {
        // 편집으로 행이 줄어 last_match가 범위를 벗어나도 패닉하지 않는다.
        let v = lines(&["a", "b"]);
        let m = find_next(2, TextPos { line: 99, col: 0 }, "a", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 0, len: 1 }));
    }

    #[test]
    fn find_prev_moves_backward_and_wraps() {
        let v = lines(&["x a", "b a", "a c"]);
        let n = 3;
        // 2행 col0 매치 앞 → 1행 col2.
        let m = find_prev(n, TextPos { line: 2, col: 0 }, "a", &opts(true, false), getter(v.clone()));
        assert_eq!(m, Some(Match { line: 1, col: 2, len: 1 }));
        // 첫 매치(0행 col2) 앞에서 찾으면 마지막 매치(2행 col0)로 감싼다.
        let m = find_prev(n, TextPos { line: 0, col: 2 }, "a", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 2, col: 0, len: 1 }));
    }

    #[test]
    fn find_prev_picks_last_match_on_the_line() {
        // 같은 행에 여러 매치가 있으면 from보다 앞쪽 중 **가장 뒤**를 준다.
        let v = lines(&["a a a"]);
        let m = find_prev(1, TextPos { line: 0, col: 4 }, "a", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_prev_single_match_wraps_to_itself() {
        let v = lines(&["zzz", "hit", "zzz"]);
        let m = find_prev(3, TextPos { line: 1, col: 0 }, "hit", &opts(true, false), getter(v));
        assert_eq!(m, Some(Match { line: 1, col: 0, len: 3 }));
    }

    #[test]
    fn find_prev_returns_none_when_absent() {
        let v = lines(&["a", "b"]);
        assert_eq!(
            find_prev(2, TextPos { line: 1, col: 0 }, "zzz", &opts(true, false), getter(v)),
            None
        );
    }

    #[test]
    fn find_prev_empty_document_is_none() {
        assert_eq!(
            find_prev(0, TextPos { line: 0, col: 0 }, "a", &opts(true, false), |_| None),
            None
        );
    }

    /// find_next를 반복하면 문서의 모든 매치를 정확히 한 번씩 돌고 제자리로
    /// 돌아온다 — 제자리걸음(같은 매치 반복)과 건너뜀이 둘 다 없다는 뜻.
    #[test]
    fn find_next_cycles_through_all_matches_once() {
        let v = lines(&["a x a", "y", "a"]);
        let n = 3;
        let mut pos = TextPos { line: 0, col: 0 };
        let mut seen = Vec::new();
        for _ in 0..3 {
            let m = find_next(n, pos, "a", &opts(true, false), getter(v.clone())).unwrap();
            seen.push((m.line, m.col));
            pos = TextPos { line: m.line, col: m.col };
        }
        assert_eq!(seen, vec![(0, 4), (2, 0), (0, 0)]);
    }

    #[test]
    fn replace_all_returns_only_changed_lines() {
        let v = lines(&["a", "b", "a", "c"]);
        let (changed, total) = replace_all(&v, "a", "Z", &opts(true, false));
        assert_eq!(changed, vec![(0, "Z".to_string()), (2, "Z".to_string())]);
        assert_eq!(total, 2);
    }

    #[test]
    fn replace_all_counts_total() {
        // 한 행에 여러 개 있으면 그만큼 센다.
        let v = lines(&["a a a", "b", "a"]);
        let (changed, total) = replace_all(&v, "a", "Z", &opts(true, false));
        assert_eq!(changed.len(), 2, "바뀐 행만");
        assert_eq!(total, 4, "치환 횟수는 행 수가 아니라 매치 수");
    }

    #[test]
    fn replace_all_empty_needle_changes_nothing() {
        let v = lines(&["a", "b"]);
        let (changed, total) = replace_all(&v, "", "Z", &opts(true, false));
        assert!(changed.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn replace_all_respects_options() {
        let v = lines(&["Test testing", "test"]);
        // 대소문자 무시 + 단어 단위 → "testing"은 제외, "Test"와 "test"만.
        let (changed, total) = replace_all(&v, "test", "X", &opts(false, true));
        assert_eq!(total, 2);
        assert_eq!(changed, vec![(0, "X testing".to_string()), (1, "X".to_string())]);
    }

    #[test]
    fn matching_lines_collects_row_numbers() {
        let v = lines(&["alpha", "beta hit", "gamma", "hit again"]);
        let got = matching_lines(4, "hit", &opts(true, false), getter(v));
        assert_eq!(got, vec![1, 3], "매치가 있는 행번호만 훑은 순서 그대로");
    }

    #[test]
    fn matching_lines_counts_each_row_once() {
        // 한 행에 세 번 나와도 그 행은 결과에 한 번뿐이다(행 단위 추출).
        let v = lines(&["hit hit hit", "none"]);
        let got = matching_lines(2, "hit", &opts(true, false), getter(v));
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn matching_lines_empty_needle_is_empty() {
        let v = lines(&["a", "b"]);
        assert!(matching_lines(2, "", &opts(true, false), getter(v)).is_empty());
    }

    #[test]
    fn matching_lines_skips_none_lines() {
        // 뷰 모드에서 인덱싱이 아직 닿지 않은 행은 get_line이 None을 준다 —
        // 건너뛸 뿐 그 자리에서 멈추지 않는다(뒤의 매치도 찾아야 한다).
        let got = matching_lines(4, "hit", &opts(true, false), |i| match i {
            0 => Some("hit".to_string()),
            1 => None,
            2 => None,
            3 => Some("hit too".to_string()),
            _ => None,
        });
        assert_eq!(got, vec![0, 3]);
    }

    #[test]
    fn matching_lines_respects_match_case() {
        let v = lines(&["HIT", "hit"]);
        assert_eq!(
            matching_lines(2, "hit", &opts(true, false), getter(v.clone())),
            vec![1],
            "대소문자 구분이 켜지면 소문자 행만"
        );
        assert_eq!(
            matching_lines(2, "hit", &opts(false, false), getter(v)),
            vec![0, 1],
            "꺼지면 둘 다"
        );
    }

    #[test]
    fn matching_lines_respects_whole_word() {
        let v = lines(&["testing", "a test here"]);
        assert_eq!(
            matching_lines(2, "test", &opts(true, true), getter(v.clone())),
            vec![1],
            "단어 단위면 'testing'은 매치가 아니다"
        );
        assert_eq!(
            matching_lines(2, "test", &opts(true, false), getter(v)),
            vec![0, 1]
        );
    }

    #[test]
    fn replace_all_never_introduces_newlines() {
        // 불변식 회귀: 어떤 치환문이 와도 결과 행에 개행이 없다.
        let v = lines(&["a,b", "c,a"]);
        let (changed, _) = replace_all(&v, "a", "1\n2\r3", &opts(true, false));
        for (_, text) in &changed {
            assert!(!text.contains('\n') && !text.contains('\r'));
        }
    }
}
