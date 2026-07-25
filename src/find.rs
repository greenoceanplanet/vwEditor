//! 찾기/바꾸기 순수 로직. `edit.rs`/`save.rs`와 같은 규율로 egui를 전혀
//! 알지 못한다 — UI(`app.rs`)는 여기 함수를 부르는 얇은 껍데기다.
//!
//! **모든 위치는 char 인덱스다.** 이 코드베이스의 `TextPos.col`이 char
//! 인덱스이므로(`edit.rs:74`) 바이트 인덱스를 섞으면 한글/이모지에서 커서가
//! 엉뚱한 곳으로 가고 `String` 슬라이싱이 패닉한다.

use crate::edit::TextPos;

/// 매치 범위. 서로 배타적(UI에서 라디오 3지). 기존 `whole_word: bool`을
/// 대체한다 — `false`는 `Partial`, `true`는 `WholeWord`에 대응하고, 여기에
/// 셀 전체 일치(`WholeCell`)를 더했다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchScope {
    /// 부분 일치(기본). 행 어디든 needle이 나오면 매치.
    Partial,
    /// 단어 단위. 매치 앞뒤가 단어 문자가 아닐 때만.
    WholeWord,
    /// 셀 전체 일치. 셀(필드) 전체가 needle과 정확히 같을 때만. 표 모드에서만
    /// 의미가 있고, 텍스트 모드(delim==None)에서는 "행 전체 일치"로 해석한다.
    WholeCell,
}

/// 찾기 옵션.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindOptions {
    /// 대소문자 구분.
    pub match_case: bool,
    /// 매치 범위(부분/단어/셀).
    pub scope: MatchScope,
}

impl Default for FindOptions {
    fn default() -> Self {
        FindOptions { match_case: false, scope: MatchScope::Partial }
    }
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
        if opts.scope == MatchScope::WholeWord {
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

/// 대소문자 규칙을 적용해 두 문자열이 **전체**로 같은지 비교한다. `match_case`면
/// 그대로 `==`, 아니면 양쪽 `to_lowercase()`. Whole cell 판정은 부분 일치가
/// 아니라 셀 전체가 needle과 정확히 같은지를 보므로 이 한 줄로 충분하다.
fn eq_scoped(a: &str, b: &str, match_case: bool) -> bool {
    if match_case {
        a == b
    } else {
        a.to_lowercase() == b.to_lowercase()
    }
}

/// needle의 모든 문자가 **바이트 접기(ASCII 한 바이트 접기)만으로 대소문자 무시
/// 비교가 성립하는가**. 참이면 `eq_ignore_ascii_case`(바이트)가 `eq_scoped`
/// (유니코드 `to_lowercase`)와 **같은 답**을 낸다.
///
/// **왜 `find.rs`에 있는가(Task M).** 원래 `app.rs`에 있었지만 이 판정이 묻는
/// 것은 "`folded`/`eq_scoped`의 유니코드 접기와 바이트 접기가 같은 질문인가"라
/// 접기 규칙을 정의하는 이 모듈의 소유다. 스캔 경로(`app.rs`의 `bytefast_ci_ok`)와
/// 치환 경로(`replace_cells_bytes`)가 **같은 한 벌**을 부른다 — 두 벌이 되면
/// 언젠가 갈린다. `app.rs`는 `use crate::find::query_is_case_foldable_by_bytes`로
/// 끌어다 쓴다(순수 모듈이 UI 모듈을 역참조하지 않게 하는 방향).
///
/// **왜 `is_ascii()`로는 너무 좁은가.** 예전 판정은 "비ASCII가 하나라도 있으면
/// 폴백"이었다. 그 근거("유니코드 접기는 바이트로 안전하지 않다")는 옳지만
/// 대상을 지나치게 넓게 잡았다 — **대소문자 개념이 아예 없는 문자**는 접어도
/// 자기 자신이라, ignore_case여도 바이트 비교가 match_case와 **완전히 같은
/// 질문**이 된다. 전 유니코드 프로브로 확인한 사실:
/// - 한글 음절(U+AC00~U+D7A3) 11,172자 중 `to_lowercase()`로 바뀌는 것: **0개**
/// - 그 밖의 비ASCII 중 바뀌는 것: 1,462자(`À`→`à`, `É`, `İ`, `Σ` 등 라틴/그리스/키릴)
///
/// 그래서 한글·한자·가나·숫자·기호로 이뤄진 needle은 빠른 경로를 타도 된다.
/// 실제로 이 함수가 걸러야 하는 것은 **바이트로 접을 수 없는** 비ASCII
/// 대소문자 문자뿐이다.
///
/// 판정:
/// - ASCII 문자는 ASCII 한 바이트 접기가 정확히 접으므로 항상 참
///   (`A`~`Z`는 거짓이 아니다 — 예전과 똑같이 빠른 경로를 탄다).
/// - 비ASCII 문자는 **소문자화도 대문자화도 자기 자신일 때만** 참.
///
/// **왜 소문자화만 보면 안 되는가(반드시 양방향).** 브루트포스(`eq_scoped` /
/// `folded`)는 hay와 needle을 **둘 다** 접어 비교한다. 그래서 needle이
/// `é`(이미 소문자 — `to_lowercase()`가 자기 자신)여도, 파일의 `É`가 접혀 `é`가
/// 되므로 브루트포스는 매치라고 답한다. 반면 바이트 경로는 비ASCII를 건드리지
/// 않아 `É`(0xC3 0x89)와 `é`(0xC3 0xA9)를 다른 바이트로 본다 → **위음성 = 계약
/// 위반**. 대문자화까지 자기 자신이어야 "이 문자로 접혀 오는 다른 문자가 없다"가
/// 보장된다.
///
/// **전 유니코드 전수 검증(프로브).** 이 조건(`is_ascii() || (lo==self &&
/// up==self)`)을 통과하는 비ASCII 문자 중, 다른 문자가 소문자화로 그 문자가 되는
/// 경우는 **0개**다. 반대로 소문자화만 보는 조건에는 그런 구멍이 1,453개
/// 있었다(`ß`, `à`, `á`, …). 한글 음절 11,172자·CJK 통합한자·가나는 전부 통과한다.
///
/// **구멍: 1글자씩 봐서는 다다자 확장의 "조각"을 못 잡는다.** 위 검증은 각 문자를
/// **홀로** 봤을 때 다른 한 문자가 그리로 접혀 오는지만 본다. 그런데
/// `char::to_lowercase()`는 **여러 문자로 확장**되기도 한다 — U+0130
/// `İ`(LATIN CAPITAL LETTER I WITH DOT ABOVE)의 소문자화는 **두 글자**
/// `i`+U+0307(COMBINING DOT ABOVE)이다. `i`는 ASCII라 통과, U+0307은 대소문자가
/// 없어(`lo==up==self`) 통과 — 그래서 needle `"i\u{0307}"`가 이 판정을
/// 문자 단위로는 전부 통과해 버린다. 하지만 브루트포스(`folded`)는 문서의 `İ`
/// 한 글자를 접어 정확히 이 두 글자 시퀀스를 만들어 내므로, 그 행은 브루트포스는
/// 매치이고 바이트 경로는 (`İ`의 UTF-8 바이트 `0xC4 0xB0`가 `i`+U+0307의 바이트
/// `0x69 0xCC 0x87`와 전혀 다르므로) 못 잡는다 → 위음성.
///
/// **막는 법.** "다다자 확장에 등장하는 문자"의 집합을 전 유니코드에서 구하면
/// U+0307 단 하나뿐이다(`multichar_lower_expansion_pieces_are_exactly_u0307`가
/// 이 사실 자체를 전수 검증한다) — 그래서 U+0307을 needle에서 거부하면 이
/// 구멍이 완전히 막힌다. 다른 다다자 확장이 훗날 추가되더라도(유니코드
/// 데이터는 버전마다 바뀔 수 있음) 이 함수가 쓰는 상수가 아니라 그 테스트가
/// 먼저 깨져 알려준다.
///
/// **왜 한글/한자/가나는 안전한가.** 이들은 애초에 대소문자 매핑이 없어(위 전수
/// 검증 1) 다다자 확장의 결과물도, 조각도 될 수 없다 — 이 배제 규칙 추가가
/// `인도네시아` 같은 needle을 막지 않는다.
///
/// **알려진 한계(이 함수 밖, 기존 동작).** 비ASCII → **ASCII**로 접히는 문자가
/// 유니코드 전체에 딱 하나 있다: U+212A KELVIN SIGN(`K`) → `k`. 그래서 ASCII
/// needle `k`로 U+212A가 든 행을 찾으면 브루트포스는 매치, 바이트 경로는
/// 비매치다(위음성). **K-1 이전부터 있던 구멍이고 이 판정과 무관하다** —
/// 막으려면 `k`/`K`가 든 모든 ASCII needle을 폴백으로 보내야 하는데, 흔한 글자
/// 하나 때문에 빠른 경로를 통째로 잃는 대가가 유니코드 호환 문자 하나보다
/// 훨씬 크다. 의도적으로 남겨 두고 여기 기록한다.
///
/// 빈 문자열은 참이다(모든 문자가 조건을 만족 — 공허참). 빈 needle을 막는 것은
/// 호출부(`app.rs`의 `bytefast_ci_ok`, `replace_all`의 이른 반환)이고, 그 책임을
/// 여기 겹쳐 두면 판정 두 곳이 갈린다.
pub(crate) fn query_is_case_foldable_by_bytes(query: &str) -> bool {
    query.chars().all(|c| {
        // U+0307: 다른 문자(U+0130)의 다다자 소문자 확장 "조각"으로만 등장하는
        // 유일한 문자(전수 검증: `app.rs`의 프로브 테스트) — 단독으로는 캐이스리스라
        // 이 검사가 없으면 통과해 버린다.
        c != '\u{0307}'
            && (c.is_ascii()
                || (c.to_lowercase().eq(std::iter::once(c))
                    && c.to_uppercase().eq(std::iter::once(c))))
    })
}

/// **치환 전용의 더 엄격한 판정**: ignore_case에서 바이트 접기가 유니코드 접기와
/// 답이 같음을 needle 쪽에서 **완전히** 보장하는가.
///
/// **왜 `query_is_case_foldable_by_bytes`만으로는 부족한가(치환 한정).** 그 판정에는
/// 의도적으로 남겨 둔 구멍이 하나 있다 — 비ASCII인데 소문자화하면 **ASCII가 되는**
/// 문자. 유니코드 전체에 딱 하나, **U+212A KELVIN SIGN(`K`) → `k`** 뿐이다
/// (`only_u212a_lowercases_to_ascii`가 이 사실 자체를 전수 검증한다).
/// 그래서 needle `k`는 그 판정을 통과하지만(전부 ASCII), 문서의 U+212A를
/// 브루트포스(`eq_scoped`)는 잡고 바이트 접기(`eq_ignore_ascii_case` /
/// `find_ci_ascii`)는 못 잡는다 → **위음성**.
///
/// **스캔은 이 구멍을 안고 가지만 치환은 안 된다.** 스캔(`app.rs`)의 계약은
/// "빠른 경로가 브루트포스와 같은 행 집합"이고 거기서는 흔한 글자 `k` 때문에
/// 빠른 경로를 통째로 잃는 대가가 유니코드 호환 문자 하나보다 크다고 판단해
/// 명시적으로 남겨 두었다. 반면 `replace_all`의 계약은 **"최적화 전과 비트 단위로
/// 동일"**이라 단 한 행도 어긋나면 안 된다 — 사용자의 파일이 조용히 덜 바뀐다.
/// 그래서 치환 쪽만 `k`/`K`가 든 needle을 폴백으로 보낸다.
/// (`replace_all_kelvin_sign_matches_reference`가 이 규칙을 지킨다.)
///
/// **대가는 정확히 말하면 이렇다.** `k`/`K`가 든 needle은 `replace_cells_bytes`
/// (바이트 셀 치환)만 잃는 게 아니라, `replace_all`의 프리필터도 `Prefilter::None`이
/// 되어 **함께 사라진다**(아래 `replace_all`의 needle_is_bytefold_exact 분기 참조) —
/// 즉 "매치 없는 행 건너뛰기" 자체가 꺼져 모든 행이 `replace_in_line`을 탄다.
/// 그래서 `king`/`key`/`check`처럼 `k`/`K`를 포함한 needle은 ignore_case에서
/// 최적화 **두 층 다** 잃고 옛 성능(사용자 파일 기준 ~21초)으로 돌아간다.
/// 그럼에도 이 폴백을 유지하는 이유는 `replace_all`의 계약이 "최적화 전과
/// 비트 단위로 동일"이기 때문이다 — 정확성이 속도보다 우선이고, `k`/`K`가 든
/// 검색어는 사용자가 겪은 실제 병목(needle `-`)을 비롯한 대부분의 검색어에 비해
/// 드물다. match_case에서는 접기 자체가 없으므로 이 제한이 적용되지 않는다.
pub(crate) fn needle_is_bytefold_exact(needle: &str, match_case: bool) -> bool {
    if match_case {
        // 접기가 없으면 바이트 비교가 곧 정확한 비교다.
        return true;
    }
    query_is_case_foldable_by_bytes(needle)
        // U+212A가 접혀 들어올 수 있는 유일한 통로를 막는다.
        && !needle.bytes().any(|b| b == b'k' || b == b'K')
}

/// ASCII 대문자 한 바이트를 소문자로 접는다. 그 밖의 바이트(숫자·기호·비ASCII
/// ≥0x80)는 그대로 둔다 — 멀티바이트 시퀀스의 바이트를 건드리면 원래 없던
/// 바이트열이 생겨 위양성/위음성이 둘 다 가능해진다.
///
/// **`find.rs`에 있는 이유(Task M).** 바로 위 `query_is_case_foldable_by_bytes`와
/// 한 쌍이다 — 그 판정이 참일 때 비로소 이 접기가 `eq_scoped`의 유니코드 접기와
/// 같은 답을 낸다. 스캔 경로(`app.rs`)와 치환 프리필터(`replace_all`)가 **같은
/// 한 벌**을 부른다.
pub(crate) fn ascii_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

/// ASCII 대소문자 무시 바이트 탐색. `needle_lower`는 **호출부가 미리 소문자로
/// 접어** 넘긴다(행마다 다시 접지 않게). hay는 비교 시점에 한 바이트씩 접으므로
/// 할당이 없다.
///
/// **왜 양쪽을 다 접는가.** 예전 판단은 "needle의 대문자 변형 하나로 memmem을
/// 돌린다"였고 그건 `Ab` 같은 혼합 대소문자를 놓쳤다(위음성). hay와 needle을
/// **둘 다** ASCII 소문자로 접으면 `Ab`/`aB`/`AB`/`ab`가 전부 같은 바이트열이
/// 되므로 ASCII 범위에서 정확하다. 비ASCII 바이트(≥0x80)는 `ascii_lower`가
/// 건드리지 않으므로 **그대로 리터럴 비교**된다 — 한글처럼 대소문자가 없는
/// needle에서는 그게 곧 정답이다(`query_is_case_foldable_by_bytes` 참조).
///
/// 첫 바이트는 `memchr2`(소문자/대문자 두 바이트)로 건너뛰어 스캔한다 —
/// 벤치에서 순진한 바이트 루프보다 눈에 띄게 빨랐다(374ms vs 408ms/2GB).
pub(crate) fn find_ci_ascii(hay: &[u8], needle_lower: &[u8]) -> Option<usize> {
    find_ci_ascii_from(hay, needle_lower, 0)
}

/// `find_ci_ascii`를 `from` 바이트 위치부터 시작한다. 모든 출현을 훑는
/// `find_ci_ascii_all`(`app.rs`, 스캔 경로 전용)이 재진입할 때 쓴다.
pub(crate) fn find_ci_ascii_from(hay: &[u8], needle_lower: &[u8], from: usize) -> Option<usize> {
    let n = needle_lower.len();
    if n == 0 || hay.len() < n || from > hay.len() - n {
        return None;
    }
    let lo = needle_lower[0];
    // 소문자로 접힌 첫 바이트의 대문자 짝. ASCII 소문자가 아니면(숫자·기호·
    // 비ASCII) 짝이 자기 자신이라 memchr 한 개로 충분하다.
    let up = if lo.is_ascii_lowercase() { lo - 32 } else { lo };
    let mut start = from;
    while start + n <= hay.len() {
        let rest = &hay[start..];
        let pos = if lo == up {
            memchr::memchr(lo, rest)
        } else {
            memchr::memchr2(lo, up, rest)
        };
        let i = start + pos?;
        if i + n > hay.len() {
            return None;
        }
        if hay[i + 1..i + n].iter().zip(&needle_lower[1..]).all(|(&h, &d)| ascii_lower(h) == d) {
            return Some(i);
        }
        start = i + 1;
    }
    None
}

/// 한 행에서 needle이 나오는 (col, len)들(char 인덱스). scope에 따라:
///
/// - `Partial`/`WholeWord`: `find_in_line`에 그대로 위임한다(delim은 무시).
/// - `WholeCell` + `delim == Some(d)`: `d`로 셀을 나눠 **셀 전체가 needle과
///   정확히 같은** 셀만 (그 셀의 char 시작 인덱스, 셀 char 길이)로 돌려준다.
/// - `WholeCell` + `delim == None`: 행 전체가 needle과 같으면 `(0, char_len)`
///   하나, 아니면 빈 결과("행 전체 일치"로 해석).
///
/// **따옴표 처리와 표시 정합성(설계 판단).** 셀 경계는 `parse::field_slice`로
/// 얻는다 — `split_fields`(csv_core)와 필드 개수·경계가 정확히 일치함이
/// `parse.rs`의 전수 테스트로 보장된다. 그런데 두 함수의 "값"은 다르다:
/// `field_slice`는 바깥 따옴표를 **포함한** 원본 슬라이스를 주고,
/// `split_fields`는 화면에 표시되는 값(따옴표 벗김, `""`→`"`)을 준다.
/// - **비교**는 사용자가 화면에서 보는 값과 맞아야 하므로 `split_fields`가 주는
///   표시 값으로 한다(따옴표 안 `bb`를 needle `bb`가 잡는다).
/// - **반환 range**는 `find_in_line`처럼 **원본 hay의 char 인덱스**여야 한다
///   (하이라이트/커서가 원본 행 위에서 움직이므로). 그래서 `field_slice`가 준
///   바이트 range(따옴표 포함)를 char 인덱스로 환산해 돌려준다. 표 모드의 셀
///   하이라이트는 다음 태스크에서 셀 텍스트에 delim=None으로 다시 부르므로,
///   따옴표를 포함한 원본 range와 표시 값의 차이가 문제되지 않는다.
pub fn find_in_line_scoped(
    hay: &str,
    needle: &str,
    opts: &FindOptions,
    delim: Option<u8>,
) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    if opts.scope != MatchScope::WholeCell {
        return find_in_line(hay, needle, opts);
    }
    match delim {
        None => {
            // 텍스트 모드: 행 전체가 needle과 같을 때만.
            if eq_scoped(hay, needle, opts.match_case) {
                vec![(0, hay.chars().count())]
            } else {
                Vec::new()
            }
        }
        Some(d) => {
            let bytes = hay.as_bytes();
            // 표시 값(따옴표 벗김)으로 비교하고, 원본 바이트 range로 char 인덱스를
            // 환산한다. 두 함수는 필드 개수·경계가 일치하므로 col로 zip할 수 있다.
            let display = crate::parse::split_fields(hay, d);
            let mut out = Vec::new();
            for (col, cell_display) in display.iter().enumerate() {
                if !eq_scoped(cell_display, needle, opts.match_case) {
                    continue;
                }
                // 원본 hay에서 이 셀의 바이트 range(따옴표 포함)를 얻어 char로 환산.
                let Some(slice) = crate::parse::field_slice(bytes, d, col) else {
                    continue;
                };
                // 슬라이스 시작의 바이트 오프셋. field_slice는 hay 내부를 가리키는
                // 슬라이스이므로 포인터 차로 오프셋을 구한다(할당 없음).
                let start_byte = slice.as_ptr() as usize - bytes.as_ptr() as usize;
                let end_byte = start_byte + slice.len();
                // 바이트 오프셋 → char 인덱스. hay를 한 번 훑어 경계를 센다.
                let start_char = hay[..start_byte].chars().count();
                let cell_char_len = hay[start_byte..end_byte].chars().count();
                out.push((start_char, cell_char_len));
            }
            out
        }
    }
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

/// 찾기/바꾸기 입력란의 이스케이프 시퀀스를 실제 문자로 푼다. 탭처럼 키보드로
/// 입력란에 직접 칠 수 없는 문자를 찾기 위한 것이다(TextEdit에서 Tab 키는
/// 포커스 이동이라 탭 문자가 들어가지 않는다).
///
/// 해석하는 것은 셋뿐이다:
/// - `\t` → 탭(U+0009)
/// - `\\` → 백슬래시 하나
/// - `\xNN` → 16진수 **정확히 2자리**가 가리키는 문자(대소문자 무관)
///
/// 그 밖의 `\?`는 백슬래시와 그 글자를 **그대로** 남긴다(관대한 해석). 문자열
/// 끝에 홀로 남은 `\`도 그대로다.
///
/// **`\n`/`\r`을 일부러 지원하지 않는다.** 이 프로그램은 문서를 행 배열로
/// 다루고 `lines[i]`에 개행이 없다는 불변식이 표 모드 행번호·정렬·저장을
/// 떠받친다(`sanitize_for_line`이 같은 이유로 존재한다). 개행은 행의 **내용**이
/// 아니라 행과 행의 **경계**이므로 애초에 "행 안에서 찾을" 대상이 아니다.
/// 그렇다고 `\n`을 **오류로 만들지도 않는다** — 위 관대 규칙에 따라 `\` + `n`
/// 두 글자로 남으므로, 사용자가 `\n`을 쳐도 그 두 글자를 찾을 뿐 아무것도
/// 깨지지 않는다. 이 "안전하게 아무 일도 일어나지 않음"이 의도된 동작이다.
///
/// **`\xNN`은 바이트가 아니라 유니코드 코드포인트다.** `char::from_u32`로 풀기
/// 때문에 `\x80`~`\xFF`는 U+0080~U+00FF(라틴-1) 문자가 된다 — 예: `\xE9`는
/// `é`이지 CP949의 어떤 바이트가 아니다. 이 코드베이스의 매칭은 전부 디코딩된
/// `String`(char 인덱스) 위에서 도므로 "바이트"라는 개념을 여기 들여올 수
/// 없고, `char`는 유니코드 스칼라라 U+0080~U+00FF가 전부 유효하다. 사용자가
/// 말한 "아스키 코드"에 해당하는 `\x00`~`\x7F` 범위에서는 두 해석이 같다.
/// (툴팁도 "character code (hex)"로 이 의미를 적는다.)
///
/// **반환이 `String`인 이유.** 백슬래시가 하나도 없으면 결과가 입력과 같으므로
/// `Cow::Borrowed`로 할당을 없앨 수 있지만, 그러면 모든 호출부가 `Cow`를 받아
/// 수명을 끌고 다녀야 한다 — 호출부는 `Highlight.query`에 넣거나(소유 필요)
/// `replace_all_in_doc`처럼 `doc`을 가변 대여하기 전에 값을 떼어 놔야 하는
/// 곳들이라 어차피 곧바로 `to_owned()`가 붙는다. 대신 백슬래시가 없으면
/// **문자별 순회 없이** `to_owned()` 한 번으로 끝내는 빠른 경로를 둔다
/// (`memchr`). 이 함수는 매 프레임이 아니라 사용자가 버튼을 누를 때만 불리므로
/// 그 이상의 최적화는 필요 없다.
pub fn unescape(s: &str) -> String {
    // 백슬래시가 없으면 해석할 것이 없다 — char 순회 없이 통째로 복사한다.
    if memchr::memchr(b'\\', s.as_bytes()).is_none() {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('t') => {
                it.next();
                out.push('\t');
            }
            Some('\\') => {
                it.next();
                out.push('\\');
            }
            Some('x') => {
                // `\x` 뒤 **정확히 2자리**만 읽는다 — `\x414`는 `\x41` + `4` =
                // `A4`다. 2자리가 안 오거나 16진수가 아니면(`\xZZ`, `\x9`, `\x`)
                // 관대 규칙대로 통째로 그대로 남긴다. 그러려면 실패했을 때
                // 되돌릴 수 있어야 하므로 반복자를 소비하기 전에 **복제본**으로
                // 먼저 시험한다(`peekable`은 한 글자만 미리 볼 수 있다).
                let mut probe = it.clone();
                probe.next(); // 'x'
                let h1 = probe.next().and_then(|c| c.to_digit(16));
                let h2 = probe.next().and_then(|c| c.to_digit(16));
                match (h1, h2) {
                    (Some(a), Some(b)) => {
                        // 0x00~0xFF는 전부 유효한 유니코드 스칼라라
                        // `from_u32`가 실패할 수 없지만, unwrap 대신 실패 시
                        // 그대로 두는 쪽으로 적어 둔다(관대 규칙과 일관).
                        match char::from_u32(a * 16 + b) {
                            Some(ch) => {
                                it = probe;
                                out.push(ch);
                            }
                            None => out.push('\\'),
                        }
                    }
                    _ => out.push('\\'),
                }
            }
            // 지원하지 않는 `\n`, 오타 `\z`, 그리고 문자열 끝의 홀로 남은 `\`.
            // 백슬래시만 흘려보내면 다음 글자는 다음 반복에서 그대로 담긴다.
            _ => out.push('\\'),
        }
    }
    out
}

/// 한 행 안에서 needle을 replacement로 모두 바꾼 새 문자열과 바뀐 횟수.
/// scope가 WholeCell이면 **셀 전체**를 replacement로 갈아 끼운다(부분이 아니라).
/// delimiter는 그대로 두고, replacement의 개행만 `sanitize_for_line`으로 막는다.
pub fn replace_in_line(
    hay: &str,
    needle: &str,
    replacement: &str,
    opts: &FindOptions,
    delim: Option<u8>,
) -> (String, usize) {
    if needle.is_empty() {
        return (hay.to_owned(), 0);
    }
    // Whole cell은 매치 구간(= 셀 전체, 따옴표 포함 범위)을 그대로 갈아 끼우면
    // 되므로 `find_in_line_scoped`가 준 range를 그대로 쓴다. Partial/WholeWord은
    // 위임되어 기존과 동일한 range를 준다.
    let hits = find_in_line_scoped(hay, needle, opts, delim);
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
    delim: Option<u8>,
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
        for (col, len) in find_in_line_scoped(&text, needle, opts, delim) {
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
    delim: Option<u8>,
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
        for (col, len) in find_in_line_scoped(&text, needle, opts, delim).into_iter().rev() {
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

/// 한 행에서 **셀 전체가 needle과 같은** 셀을 replacement로 갈아 끼운 새 문자열과
/// 바꾼 횟수. `replace_in_line`(Whole cell, `delim == Some(d)`)과 **결과가 비트
/// 단위로 같아야 한다** — 그것이 이 함수의 존재 이유이자 유일한 계약이다.
///
/// **왜 필요한가(Task M).** `replace_in_line`은 매 행 `find_in_line_scoped`를
/// 부르고, 그건 Whole cell에서 `parse::split_fields`로 **모든 필드를 `String`으로
/// 할당**한다. 1,542만 행 × 8열 = 1억 2천만 할당이라 판정에만 17.6초가 든다
/// (실제 문자열 조립은 0.4초뿐이었다). 여기서는 `memchr`로 셀 경계만 훑고
/// 바이트를 그대로 비교하므로 할당이 **출력 문자열 하나**뿐이다.
///
/// **바이트로 판정해도 되는 근거.** 행에 `"`가 하나도 없으면 csv 파싱이 개입할
/// 것이 없어 `split_fields`가 주는 표시값 == `delim`으로 자른 원본 바이트 조각이고,
/// `field_slice`가 주는 원본 range == 바로 그 조각의 range다. 즉 "셀을 delim으로
/// 자르고 통째로 갈아 끼우기"가 곧 기존 동작이다.
///
/// **폴백(`None`)을 돌려주는 두 경우.** 둘 다 "애매하면 정확한 쪽으로" 규율이다:
///
/// 1. **행에 `"`가 있으면**(`memchr`). 따옴표 셀은 표시값과 원본 바이트가 달라
///    (`"a"a`가 `aa`로 보이고, `"a,b"`는 delim을 품는다) 바이트로 셀 경계도
///    셀 값도 판정할 수 없다. needle 바이트가 행에 한 번도 안 나와도 매치일 수
///    있으므로 **"매치 없음"조차 바이트로 단정하면 안 된다** — Task I에서 실제로
///    위음성을 낸 함정이다(`app.rs`의 `cell_bytes_are_display` 주석 참조).
/// 2. **ignore_case인데 needle이 바이트로 접히지 않으면**
///    (`query_is_case_foldable_by_bytes`). `eq_ignore_ascii_case`는 ASCII만 접으니
///    `é`/`É`처럼 유니코드 접기가 필요한 needle은 `eq_scoped`와 다른 답을 낸다.
///    스캔 경로(`bytefast_ci_ok`)와 **같은 한 벌**의 판정을 부른다.
///
/// **인코딩 걱정이 여기엔 없다.** 편집 모드의 `EditBuffer.lines`는 문서 인코딩과
/// 무관하게 **항상 UTF-8**이다(`load_edit_buffer`가 진입 시 디코딩해 넣는다).
/// 그래서 스캔 경로가 CP949 트레일 바이트(0x41~0xFE가 ASCII 대문자와 겹침)나
/// UTF-16 2바이트 코드유닛 때문에 폴백해야 했던 이유가 이 함수엔 적용되지 않는다.
/// UTF-8은 self-synchronizing이라 단일 바이트 ASCII delim이 멀티바이트 문자
/// 중간에 걸릴 수도 없다.
///
/// **`unsafe { from_utf8_unchecked }`를 쓰지 않는다.** 셀 경계는 위 이유로 항상
/// UTF-8 문자 경계이므로 `&line[start..end]`(바이트 인덱스 `str` 슬라이싱)가
/// 그냥 안전하다. 경계 검사 비용은 실측에서 노이즈였다(리포트 참조).
///
/// **버퍼는 진입하자마자 잡는다(지연 할당이 아니라).** 처음엔 첫 매치 전까지
/// 할당을 미루면 프리필터가 필요 없겠다고 봤지만, 실측이 그 판단을 뒤집었다 —
/// 사용자 파일에서 지연 할당(프리필터 없음) 2,267ms vs 프리필터+즉시 할당
/// 1,560ms. `memmem`으로 행을 통째로 건너뛰는 것이 셀 루프를 끝까지 도는 것보다
/// 훨씬 싸다. 그래서 "매치 없는 행 건너뛰기"는 호출부의 프리필터가 맡고, 이
/// 함수는 **프리필터를 통과한 행만** 받아 단순하게 조립한다(리포트 참조).
pub(crate) fn replace_cells_bytes(
    line: &str,
    needle: &str,
    rep: &str,
    delim: u8,
    match_case: bool,
) -> Option<(String, usize)> {
    let lb = line.as_bytes();
    // 1. 따옴표 행은 표시값 != 원본 바이트 → 바이트로는 아무것도 단정할 수 없다.
    if memchr::memchr(b'"', lb).is_some() {
        return None;
    }
    // 1-bis. 홀로 있는 `\r`/`\n`이 낀 행도 마찬가지로 물러난다. `csv_core`는
    // 이 두 바이트를 **레코드 종결자**로 취급해 `split_fields`가 그 뒤를 다음
    // 레코드로 넘겨 버리므로(예: "a\r,b" → 셀 하나 "a"), 여기서 델리미터만
    // 보고 그대로 두 셀("a\r", "b")로 나누면 `field_slice`/`split_fields`가
    // 세는 셀 경계와 어긋난다. `load_edit_buffer`는 `\n` 앞의 `\r`과 EOF의
    // `\r`만 벗기므로 행 중간의 홀로 있는 CR은 그대로 살아 여기까지 온다
    // (Task M 리뷰: `h1,h2\na\r,b\nc,d\n` 로드 후 needle `a`/`b` 양쪽에서
    // 위음성/위양성이 실측됨). quote 가드와 같은 등급의 안전장치이므로 같은
    // 자리에 둔다.
    if memchr::memchr2(b'\r', b'\n', lb).is_some() {
        return None;
    }
    // 2. ignore_case인데 바이트 접기가 유니코드 접기와 답이 갈릴 수 있는 needle
    //    (`é` 같은 접히는 문자, 그리고 U+212A가 접혀 오는 `k`) → 폴백.
    if !needle_is_bytefold_exact(needle, match_case) {
        return None;
    }
    let nb = needle.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut count = 0usize;
    let mut start = 0usize; // 지금 보고 있는 셀의 시작 바이트
    loop {
        let end = match memchr::memchr(delim, &lb[start..]) {
            Some(p) => start + p,
            None => lb.len(),
        };
        let cell = &lb[start..end];
        // 셀 전체가 needle과 같은가. ASCII 접기가 곧 정답임은 위 2번이 보장한다.
        let hit = if match_case {
            cell == nb
        } else {
            cell.eq_ignore_ascii_case(nb)
        };
        if hit {
            out.push_str(rep);
            count += 1;
        } else {
            // 안전 슬라이싱. 셀 경계는 항상 UTF-8 문자 경계라 패닉하지 않는다.
            out.push_str(&line[start..end]);
        }
        if end >= lb.len() {
            break;
        }
        out.push(delim as char);
        start = end + 1;
    }
    Some((out, count))
}

/// 치환 프리필터가 쓰는 바이트 탐색 방식. `replace_all`이 needle과 옵션을 보고
/// **한 번** 정하고, 행마다 이걸 그대로 적용한다(행마다 다시 판단하지 않게).
enum Prefilter {
    /// match_case: `memmem`이 곧 정확한 부분 문자열 탐색이다.
    ///
    /// `Box`인 이유는 순전히 크기 균형이다 — `Finder`가 288바이트라 다른 변형
    /// (24바이트/0바이트)과 차이가 커진다(`clippy::large_enum_variant`). 이
    /// 값은 `replace_all` **한 번에 하나만** 만들어 루프 밖에 두므로 힙 할당이
    /// 행마다 생기지 않는다(호출당 1회).
    Exact(Box<memchr::memmem::Finder<'static>>),
    /// ignore_case + 바이트로 접히는 needle: 스캔 경로와 **같은** `find_ci_ascii`.
    /// 미리 소문자로 접은 needle 바이트를 들고 다닌다.
    CaseFold(Vec<u8>),
    /// 프리필터를 걸 수 없다(ignore_case인데 유니코드 접기가 필요한 needle 등).
    /// 모든 행이 통과한다 — 최적화를 포기할 뿐 결과는 언제나 옳다.
    None,
}

/// 이 행을 **건너뛰어도 되는가**(= needle이 이 행에 있을 수 없는가).
///
/// **절대 규율: 위음성 금지.** 참을 돌려주면 그 행은 `replace_in_line`도
/// `replace_cells_bytes`도 보지 못하고 결과에서 빠진다. 그러니 "확실히 매치가
/// 없다"를 증명할 수 있을 때만 참이어야 한다. 반대로 거짓(=통과)은 언제나
/// 안전하다 — 뒤따르는 정밀 판정이 다시 거른다. 애매하면 거짓이 정답이다.
///
/// 판단 세 가지가 겹쳐 있다:
///
/// 1. **따옴표 행은 절대 건너뛰지 않는다**(`quote_sensitive`가 참일 때).
///    Whole cell의 비교 대상은 파일 바이트가 아니라 **표시값**이라, `"a"a`는
///    표시값이 `aa`인데 바이트에는 `aa`가 없다 — 바이트로 "매치 없음"이라
///    단정하면 그 행을 통째로 잃는다(Task I에서 실제로 낸 위음성).
///    Partial/WholeWord는 비교 대상이 행 원문 그대로라 이 함정이 없으므로
///    `quote_sensitive`가 거짓이고, 따옴표가 있어도 바이트로 판정해도 된다.
/// 2. **홀로 있는 `\r`/`\n`이 낀 행도 `quote_sensitive`일 때 건너뛰지 않는다.**
///    `csv_core`(`split_fields`)는 이 두 바이트를 레코드 종결자로 삼아 그 뒤를
///    잘라 버리는데, 같은 열(col)의 바이트 범위를 주는 `field_slice`는 CR/LF를
///    전혀 모르고 델리미터만 본다 — 그래서 표시값 비교(`split_fields`)와 실제
///    치환 범위(`field_slice`)가 이미 서로 다른 규칙으로 움직인다(Task M 리뷰).
///    이 두 함수의 불일치가 어떤 입력에서 정확히 어떻게 프리필터를 속이는지
///    분석만으로 완전히 배제하기보다, 따옴표와 **같은 등급의 구조적 위험**으로
///    보고 무조건 통과시키는 쪽을 택한다 — "애매하면 거짓" 규율 그대로다.
///    (`replace_cells_bytes`도 같은 이유로 이 행에서 물러난다.)
/// 3. **needle 바이트가 한 번도 안 나오면** 매치가 있을 수 없다. `Exact`는
///    match_case라 그대로, `CaseFold`는 양쪽을 ASCII 소문자로 접어 비교하므로
///    `Ab`/`aB`/`AB`도 전부 잡는다(단순 `memmem`으로는 놓쳐 위음성이 난다).
///    `None`이면 판단을 포기하고 통과시킨다.
///
/// 프로덕션(`replace_all`)과 테스트가 **이 함수 하나**를 공유한다 — 판정을
/// 루프 안에 인라인으로 적어 두면 그것을 뒤집어도 테스트가 자기 사본만 보고
/// 통과한다(이 코드베이스의 `extract_plan`·`classify_cell_hit`과 같은 규율).
fn replace_row_can_skip(line: &str, pre: &Prefilter, quote_sensitive: bool) -> bool {
    let lb = line.as_bytes();
    if quote_sensitive && memchr::memchr(b'"', lb).is_some() {
        // 따옴표 행: 표시값 != 바이트라 아무것도 단정할 수 없다.
        return false;
    }
    if quote_sensitive && memchr::memchr2(b'\r', b'\n', lb).is_some() {
        // CR/LF 행: split_fields(표시값)와 field_slice(치환 범위)가 CR/LF를
        // 다르게 취급해 서로 어긋난다 — 따옴표와 같은 등급의 구조적 위험이라
        // 프리필터가 판단을 내리지 않고 통과시킨다.
        return false;
    }
    match pre {
        Prefilter::Exact(f) => f.find(lb).is_none(),
        Prefilter::CaseFold(needle_lower) => find_ci_ascii(lb, needle_lower).is_none(),
        Prefilter::None => false,
    }
}

/// 모든 행에서 needle을 replacement로 바꾼다. 바뀐 행만 (행번호, 새 텍스트)로
/// 반환한다 — 호출부가 그것만 버퍼에 반영하고 undo에 기록할 수 있게.
/// 총 치환 횟수도 함께 반환.
///
/// 편집 모드 전용이다(뷰 모드는 버퍼가 없어 바꿀 수 없다). 그래서 `&[String]`을
/// 직접 받는다. 바뀌지 않은 행을 결과에 넣지 않는 것이 핵심이다 — 200만 행에서
/// 10행만 바뀌었는데 200만 개를 돌려주면 undo 스택이 파일 전체를 복제한다.
///
/// **최적화는 전부 이 함수 안에 있다(Task M).** 호출부(`app.rs`의
/// `replace_all_in_doc`)는 undo/dirty 규율만 신경 쓰면 되도록, 빠른 경로 선택은
/// 여기서 끝낸다. 어느 경로를 타든 결과(`Vec<(usize, String)>`과 총 횟수)는
/// **행 단위 `replace_in_line`을 전 행에 돌린 것과 비트 단위로 같다** —
/// `replace_all_bytes_matches_reference`가 옛 경로를 오라클로 두고 이를 고정한다.
///
/// 층이 둘이다:
///
/// 1. **프리필터(모든 스코프)** — `replace_row_can_skip`. 매치가 있을 수 없는
///    행은 아무 함수도 부르지 않고 건너뛴다. 예전에는 그런 행에도
///    `hay.to_owned()` 복사본이 생겼다(`replace_in_line`의 이른 반환).
/// 2. **바이트 치환(Whole cell + `delim == Some(d)`)** — `replace_cells_bytes`.
///    여기가 실제 병목이었다. 셀 경계를 바이트로 훑을 수 있어 `split_fields`의
///    필드별 `String` 할당(사용자 파일에서 1억 2천만 개)을 통째로 없앨 수 있는
///    유일한 스코프다. `None`(따옴표 행 / 접히지 않는 needle)이면 그 행만
///    `replace_in_line`으로 폴백한다.
///
/// **왜 Partial/WholeWord는 바이트 치환까지 가지 않는가(범위 결정).** 이득이
/// 프리필터에서 거의 다 나오기 때문이다. Whole cell의 비용은 "매치 없는 행에도
/// 필드 전체를 `String`으로 할당"하는 데서 왔지만, Partial의 `find_in_line`은
/// needle이 없으면 `folded()` 두 벌만 만들고 끝난다 — 그 행들을 프리필터로
/// 건너뛰면 남는 비용은 **진짜 매치 행**에만 붙고 그건 어차피 치환해야 하는
/// 행이다. WholeWord는 단어 경계가 유니코드 판정(`is_word_char`)이라 바이트로
/// 옮기면 스캔 경로가 이미 폴백으로 처리하는 문제를 치환 쪽에 새로 들여온다.
/// 얻는 것보다 잃는 위험이 크다.
///
/// **프리필터의 따옴표 민감도는 스코프에 달렸다.** Whole cell만 표시값과 바이트가
/// 갈리므로 그때만 따옴표 행을 통과시킨다(`quote_sensitive`). Partial/WholeWord는
/// 행 원문을 그대로 비교하므로 따옴표가 있어도 바이트 판정이 정확하다 —
/// 여기서 무조건 통과시키면 따옴표가 흔한 CSV에서 프리필터가 통째로 무력해진다.
pub fn replace_all(
    lines: &[String],
    needle: &str,
    replacement: &str,
    opts: &FindOptions,
    delim: Option<u8>,
) -> (Vec<(usize, String)>, usize) {
    if needle.is_empty() {
        return (Vec::new(), 0);
    }
    // Whole cell + 표 모드면 바이트 치환을 시도한다. 그 외는 프리필터만 얹는다.
    let cell_delim = match (opts.scope, delim) {
        (MatchScope::WholeCell, Some(d)) => Some(d),
        _ => None,
    };
    // 따옴표 함정은 Whole cell(표시값 비교)에만 있다 — 위 주석 참조.
    let quote_sensitive = opts.scope == MatchScope::WholeCell;
    // 치환문 개행 방어는 `replace_in_line`과 **같은 규칙**이어야 하므로 같은
    // 함수를 쓴다. 행마다 부르지 않고 한 번만 접어 둔다.
    let rep = sanitize_for_line(replacement);
    // 프리필터 방식은 needle과 옵션으로 한 번만 정한다.
    let pre = if opts.match_case {
        Prefilter::Exact(Box::new(memchr::memmem::Finder::new(needle.as_bytes()).into_owned()))
    } else if needle_is_bytefold_exact(needle, false) {
        // 스캔 경로와 **같은 탐색**(`find_ci_ascii`)을 쓰되, 판정은 치환의 더
        // 엄격한 쪽(`needle_is_bytefold_exact`)이다 — 계약이 다르기 때문.
        Prefilter::CaseFold(needle.bytes().map(ascii_lower).collect())
    } else {
        // `é`(유니코드 접기 필요)나 `k`/`K`(U+212A가 접혀 옴) — 건너뛸 근거가
        // 없다. **이 분기는 프리필터뿐 아니라 `replace_cells_bytes`도 함께
        // 잃는다**(`needle_is_bytefold_exact`가 같은 판정을 그 함수 진입부에서도
        // 쓴다) — 즉 `king`/`key`/`check`처럼 `k`/`K`가 든 needle은 ignore_case에서
        // 최적화 두 층이 전부 꺼져 옛 성능(~21초, 사용자 파일 기준)으로 돌아간다.
        // 대가가 크지만 정확성(비트 단위 동일 계약)이 우선이라 감수한다
        // (`needle_is_bytefold_exact`의 doc 참조).
        Prefilter::None
    };
    let mut changed = Vec::new();
    let mut total = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if replace_row_can_skip(line, &pre, quote_sensitive) {
            continue;
        }
        if let Some(d) = cell_delim {
            if let Some((new, n)) = replace_cells_bytes(line, needle, &rep, d, opts.match_case) {
                if n > 0 {
                    total += n;
                    changed.push((i, new));
                }
                continue;
            }
            // 폴백: 따옴표 행이거나 바이트로 접히지 않는 needle.
        }
        let (new, n) = replace_in_line(line, needle, replacement, opts, delim);
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
/// 프로덕션 호출부는 없다 — 추출까지 `scan_all_matches`(바이트 빠른 경로)로
/// 옮겨 갔기 때문이다. 그래도 지우지 않는다: 이 함수가 바이트 스캔의 **정답을
/// 정의하는 기준**이고, "빠른 경로 결과 == 이 함수 결과"라는 절대 계약을 여러
/// 테스트가 이 함수로 검증한다. 지우면 그 계약을 확인할 수단이 사라진다.
#[allow(dead_code)]
pub fn matching_lines(
    line_count: usize,
    needle: &str,
    opts: &FindOptions,
    delim: Option<u8>,
    get_line: impl Fn(usize) -> Option<String>,
) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..line_count {
        let Some(text) = get_line(i) else { continue };
        if !find_in_line_scoped(&text, needle, opts, delim).is_empty() {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::load_edit_buffer;
    use crate::parse::Encoding;
    use crate::source::Source;

    /// 기존 테스트가 `(match_case, whole_word)`로 옵션을 만들던 관습을 유지한다 —
    /// `whole_word: true`는 `WholeWord`, `false`는 `Partial`로 이관(의미 불변).
    fn opts(match_case: bool, whole_word: bool) -> FindOptions {
        let scope = if whole_word { MatchScope::WholeWord } else { MatchScope::Partial };
        FindOptions { match_case, scope }
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
        let (s, n) = replace_in_line("a b a b", "a", "X", &opts(true, false), None);
        assert_eq!(s, "X b X b");
        assert_eq!(n, 2);
    }

    #[test]
    fn replace_in_line_no_match_is_unchanged() {
        let (s, n) = replace_in_line("abc", "zzz", "X", &opts(true, false), None);
        assert_eq!(s, "abc");
        assert_eq!(n, 0);
    }

    #[test]
    fn replace_in_line_empty_needle_is_noop() {
        let (s, n) = replace_in_line("abc", "", "X", &opts(true, false), None);
        assert_eq!(s, "abc");
        assert_eq!(n, 0);
    }

    #[test]
    fn replace_in_line_sanitizes_newline_in_replacement() {
        // lines[i] 불변식 회귀 테스트 — 치환문의 개행은 공백이 된다.
        let (s, n) = replace_in_line("a,b", "b", "x\ny", &opts(true, false), None);
        assert_eq!(s, "a,x y");
        assert_eq!(n, 1);
        assert!(!s.contains('\n'));
        let (s2, _) = replace_in_line("a", "a", "p\r\nq", &opts(true, false), None);
        assert_eq!(s2, "p  q", "\\r\\n 두 문자가 각각 공백으로");
        assert!(!s2.contains('\r'));
    }

    #[test]
    fn replace_in_line_longer_replacement() {
        // 치환문이 원문보다 길어도 뒤 매치의 인덱스가 어긋나면 안 된다.
        let (s, n) = replace_in_line("a-a-a", "a", "LONG", &opts(true, false), None);
        assert_eq!(s, "LONG-LONG-LONG");
        assert_eq!(n, 3);
    }

    #[test]
    fn replace_in_line_multibyte_slicing_is_safe() {
        // char 인덱스를 바이트로 옮기지 않으면 여기서 패닉하거나 깨진다.
        let (s, n) = replace_in_line("가나ABC가나", "ABC", "다", &opts(false, false), None);
        assert_eq!(s, "가나다가나");
        assert_eq!(n, 1);
    }

    #[test]
    fn replace_in_line_case_insensitive_keeps_surrounding_text() {
        let (s, n) = replace_in_line("Hello HELLO", "hello", "hi", &opts(false, false), None);
        assert_eq!(s, "hi hi");
        assert_eq!(n, 2);
    }

    #[test]
    fn find_next_moves_forward_and_wraps() {
        let v = lines(&["x a", "b a", "a c"]);
        let n = 3;
        // 0행 col2 매치 다음 → 1행 col2.
        let m = find_next(n, TextPos { line: 0, col: 2 }, "a", &opts(true, false), None, getter(v.clone()));
        assert_eq!(m, Some(Match { line: 1, col: 2, len: 1 }));
        // 마지막 매치(2행 col0) 뒤에서 찾으면 처음 매치(0행 col2)로 감싼다.
        let m = find_next(n, TextPos { line: 2, col: 0 }, "a", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_next_does_not_return_from_position() {
        // from이 매치 자리(0,0)면 그 다음 매치(0,2)를 준다.
        let v = lines(&["a a a"]);
        let m = find_next(1, TextPos { line: 0, col: 0 }, "a", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_next_single_match_wraps_to_itself() {
        // 매치가 하나뿐이고 그게 from 자리면 한 바퀴 돌아 자기 자신(에디터 관례).
        let v = lines(&["zzz", "hit", "zzz"]);
        let m = find_next(3, TextPos { line: 1, col: 0 }, "hit", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 1, col: 0, len: 3 }));
    }

    #[test]
    fn find_next_returns_none_when_absent() {
        let v = lines(&["a", "b", "c"]);
        let m = find_next(3, TextPos { line: 0, col: 0 }, "zzz", &opts(true, false), None, getter(v));
        assert_eq!(m, None);
    }

    #[test]
    fn find_next_skips_lines_that_return_none() {
        // 뷰 모드에서 인덱싱이 안 끝난 행은 get_line이 None을 준다.
        let m = find_next(4, TextPos { line: 0, col: 0 }, "hit", &opts(true, false), None, |i| {
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
            find_next(0, TextPos { line: 0, col: 0 }, "a", &opts(true, false), None, |_| None),
            None
        );
    }

    #[test]
    fn find_next_empty_needle_is_none() {
        let v = lines(&["abc"]);
        assert_eq!(
            find_next(1, TextPos { line: 0, col: 0 }, "", &opts(true, false), None, getter(v)),
            None
        );
    }

    #[test]
    fn find_next_from_beyond_last_line_is_clamped() {
        // 편집으로 행이 줄어 last_match가 범위를 벗어나도 패닉하지 않는다.
        let v = lines(&["a", "b"]);
        let m = find_next(2, TextPos { line: 99, col: 0 }, "a", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 0, len: 1 }));
    }

    #[test]
    fn find_prev_moves_backward_and_wraps() {
        let v = lines(&["x a", "b a", "a c"]);
        let n = 3;
        // 2행 col0 매치 앞 → 1행 col2.
        let m = find_prev(n, TextPos { line: 2, col: 0 }, "a", &opts(true, false), None, getter(v.clone()));
        assert_eq!(m, Some(Match { line: 1, col: 2, len: 1 }));
        // 첫 매치(0행 col2) 앞에서 찾으면 마지막 매치(2행 col0)로 감싼다.
        let m = find_prev(n, TextPos { line: 0, col: 2 }, "a", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 2, col: 0, len: 1 }));
    }

    #[test]
    fn find_prev_picks_last_match_on_the_line() {
        // 같은 행에 여러 매치가 있으면 from보다 앞쪽 중 **가장 뒤**를 준다.
        let v = lines(&["a a a"]);
        let m = find_prev(1, TextPos { line: 0, col: 4 }, "a", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 0, col: 2, len: 1 }));
    }

    #[test]
    fn find_prev_single_match_wraps_to_itself() {
        let v = lines(&["zzz", "hit", "zzz"]);
        let m = find_prev(3, TextPos { line: 1, col: 0 }, "hit", &opts(true, false), None, getter(v));
        assert_eq!(m, Some(Match { line: 1, col: 0, len: 3 }));
    }

    #[test]
    fn find_prev_returns_none_when_absent() {
        let v = lines(&["a", "b"]);
        assert_eq!(
            find_prev(2, TextPos { line: 1, col: 0 }, "zzz", &opts(true, false), None, getter(v)),
            None
        );
    }

    #[test]
    fn find_prev_empty_document_is_none() {
        assert_eq!(
            find_prev(0, TextPos { line: 0, col: 0 }, "a", &opts(true, false), None, |_| None),
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
            let m = find_next(n, pos, "a", &opts(true, false), None, getter(v.clone())).unwrap();
            seen.push((m.line, m.col));
            pos = TextPos { line: m.line, col: m.col };
        }
        assert_eq!(seen, vec![(0, 4), (2, 0), (0, 0)]);
    }

    #[test]
    fn replace_all_returns_only_changed_lines() {
        let v = lines(&["a", "b", "a", "c"]);
        let (changed, total) = replace_all(&v, "a", "Z", &opts(true, false), None);
        assert_eq!(changed, vec![(0, "Z".to_string()), (2, "Z".to_string())]);
        assert_eq!(total, 2);
    }

    #[test]
    fn replace_all_counts_total() {
        // 한 행에 여러 개 있으면 그만큼 센다.
        let v = lines(&["a a a", "b", "a"]);
        let (changed, total) = replace_all(&v, "a", "Z", &opts(true, false), None);
        assert_eq!(changed.len(), 2, "바뀐 행만");
        assert_eq!(total, 4, "치환 횟수는 행 수가 아니라 매치 수");
    }

    #[test]
    fn replace_all_empty_needle_changes_nothing() {
        let v = lines(&["a", "b"]);
        let (changed, total) = replace_all(&v, "", "Z", &opts(true, false), None);
        assert!(changed.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn replace_all_respects_options() {
        let v = lines(&["Test testing", "test"]);
        // 대소문자 무시 + 단어 단위 → "testing"은 제외, "Test"와 "test"만.
        let (changed, total) = replace_all(&v, "test", "X", &opts(false, true), None);
        assert_eq!(total, 2);
        assert_eq!(changed, vec![(0, "X testing".to_string()), (1, "X".to_string())]);
    }

    #[test]
    fn matching_lines_collects_row_numbers() {
        let v = lines(&["alpha", "beta hit", "gamma", "hit again"]);
        let got = matching_lines(4, "hit", &opts(true, false), None, getter(v));
        assert_eq!(got, vec![1, 3], "매치가 있는 행번호만 훑은 순서 그대로");
    }

    #[test]
    fn matching_lines_counts_each_row_once() {
        // 한 행에 세 번 나와도 그 행은 결과에 한 번뿐이다(행 단위 추출).
        let v = lines(&["hit hit hit", "none"]);
        let got = matching_lines(2, "hit", &opts(true, false), None, getter(v));
        assert_eq!(got, vec![0]);
    }

    #[test]
    fn matching_lines_empty_needle_is_empty() {
        let v = lines(&["a", "b"]);
        assert!(matching_lines(2, "", &opts(true, false), None, getter(v)).is_empty());
    }

    #[test]
    fn matching_lines_skips_none_lines() {
        // 뷰 모드에서 인덱싱이 아직 닿지 않은 행은 get_line이 None을 준다 —
        // 건너뛸 뿐 그 자리에서 멈추지 않는다(뒤의 매치도 찾아야 한다).
        let got = matching_lines(4, "hit", &opts(true, false), None, |i| match i {
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
            matching_lines(2, "hit", &opts(true, false), None, getter(v.clone())),
            vec![1],
            "대소문자 구분이 켜지면 소문자 행만"
        );
        assert_eq!(
            matching_lines(2, "hit", &opts(false, false), None, getter(v)),
            vec![0, 1],
            "꺼지면 둘 다"
        );
    }

    #[test]
    fn matching_lines_respects_whole_word() {
        let v = lines(&["testing", "a test here"]);
        assert_eq!(
            matching_lines(2, "test", &opts(true, true), None, getter(v.clone())),
            vec![1],
            "단어 단위면 'testing'은 매치가 아니다"
        );
        assert_eq!(
            matching_lines(2, "test", &opts(true, false), None, getter(v)),
            vec![0, 1]
        );
    }

    #[test]
    fn replace_all_never_introduces_newlines() {
        // 불변식 회귀: 어떤 치환문이 와도 결과 행에 개행이 없다.
        let v = lines(&["a,b", "c,a"]);
        let (changed, _) = replace_all(&v, "a", "1\n2\r3", &opts(true, false), None);
        for (_, text) in &changed {
            assert!(!text.contains('\n') && !text.contains('\r'));
        }
    }

    // ---- MatchScope / Whole cell (E1) ----

    /// WholeCell 옵션을 만드는 헬퍼.
    fn cell_opts(match_case: bool) -> FindOptions {
        FindOptions { match_case, scope: MatchScope::WholeCell }
    }

    #[test]
    fn match_scope_default_is_partial() {
        assert_eq!(FindOptions::default().scope, MatchScope::Partial);
    }

    #[test]
    fn whole_cell_matches_exact_cell_only() {
        // "a,bb,ccc"에서 delim=',', needle="bb" → 셀 1(col 2, len 2)만.
        let hits = find_in_line_scoped("a,bb,ccc", "bb", &cell_opts(true), Some(b','));
        assert_eq!(hits, vec![(2, 2)]);
        // needle="b"는 부분 매치일 뿐 셀 전체가 아니므로 매치 없음.
        assert!(find_in_line_scoped("a,bb,ccc", "b", &cell_opts(true), Some(b',')).is_empty());
    }

    #[test]
    fn whole_cell_char_index_with_hangul() {
        // "가,나다,x"의 char 열: 가(0) ,(1) 나(2) 다(3) ,(4) x(5).
        // needle="나다" → 셀 1의 시작은 char 2, 길이 2(바이트가 아니다).
        let hits = find_in_line_scoped("가,나다,x", "나다", &cell_opts(true), Some(b','));
        assert_eq!(hits, vec![(2, 2)]);
        // 돌려준 col/len으로 원본을 char 단위로 자르면 정확히 셀 값이어야 한다.
        let chars: Vec<char> = "가,나다,x".chars().collect();
        let (col, len) = hits[0];
        let got: String = chars[col..col + len].iter().collect();
        assert_eq!(got, "나다");
    }

    #[test]
    fn whole_cell_ignore_case() {
        // "A,BB,c"에서 needle="bb" + ignore_case → 셀 1 매치.
        let hits = find_in_line_scoped("A,BB,c", "bb", &cell_opts(false), Some(b','));
        assert_eq!(hits, vec![(2, 2)]);
        // match_case면 매치 없음.
        assert!(find_in_line_scoped("A,BB,c", "bb", &cell_opts(true), Some(b',')).is_empty());
    }

    #[test]
    fn whole_cell_text_mode_matches_whole_line() {
        // delim=None: 행 전체가 needle과 같을 때만.
        let hits = find_in_line_scoped("hello", "hello", &cell_opts(true), None);
        assert_eq!(hits, vec![(0, 5)]);
        // 부분만 같으면 매치 없음.
        assert!(find_in_line_scoped("hello world", "hello", &cell_opts(true), None).is_empty());
        // ignore_case도 행 전체 규칙을 따른다.
        let hits = find_in_line_scoped("HELLO", "hello", &cell_opts(false), None);
        assert_eq!(hits, vec![(0, 5)]);
    }

    #[test]
    fn whole_cell_respects_quotes() {
        // 따옴표 안 콤마는 셀을 쪼개지 않는다(field_slice/split_fields 재사용 확인).
        // `"a,b",c` → 셀 0 = 표시값 "a,b", 셀 1 = "c".
        // needle "a,b"는 표시값(따옴표 벗김)과 같으므로 셀 0을 잡는다.
        // 반환 range는 원본(따옴표 포함) 슬라이스 기준: char 0..5 (`"a,b"`).
        let hits = find_in_line_scoped("\"a,b\",c", "a,b", &cell_opts(true), Some(b','));
        assert_eq!(hits, vec![(0, 5)]);
        // 따옴표 안 콤마 때문에 needle "a"는 셀 전체가 아니라 매치 없음.
        assert!(find_in_line_scoped("\"a,b\",c", "a", &cell_opts(true), Some(b',')).is_empty());
        // 셀 1은 "c" 그대로.
        assert_eq!(
            find_in_line_scoped("\"a,b\",c", "c", &cell_opts(true), Some(b',')),
            vec![(6, 1)]
        );
    }

    #[test]
    fn scoped_partial_delegates_to_find_in_line() {
        // Partial/WholeWord은 delim과 무관하게 find_in_line과 완전히 같다.
        let hay = "abc abc x";
        for sc in [MatchScope::Partial, MatchScope::WholeWord] {
            let o = FindOptions { match_case: true, scope: sc };
            assert_eq!(
                find_in_line_scoped(hay, "abc", &o, Some(b',')),
                find_in_line(hay, "abc", &o),
                "scope {sc:?}는 delim을 무시하고 find_in_line에 위임"
            );
            assert_eq!(
                find_in_line_scoped(hay, "abc", &o, None),
                find_in_line(hay, "abc", &o),
            );
        }
    }

    #[test]
    fn find_next_respects_whole_cell() {
        // "a,bb"(0행), "bb,a"(1행)에서 needle="bb" WholeCell → 셀 전체가 bb인 곳만.
        let v = lines(&["a,bb", "bb,a"]);
        let o = cell_opts(true);
        // 0행 col2(bb) 다음 → 1행 col0(bb).
        let m = find_next(2, TextPos { line: 0, col: 2 }, "bb", &o, Some(b','), getter(v.clone()));
        assert_eq!(m, Some(Match { line: 1, col: 0, len: 2 }));
        // 부분 매치("b")는 WholeCell에서 잡히지 않는다.
        let m = find_next(2, TextPos { line: 0, col: 0 }, "b", &o, Some(b','), getter(v));
        assert_eq!(m, None);
    }

    #[test]
    fn replace_all_whole_cell_replaces_entire_cell() {
        // 셀 전체가 needle과 같은 셀만, 셀 전체를 replacement로 바꾼다.
        let v = lines(&["a,bb,bbc", "bb,x"]);
        let (changed, total) = replace_all(&v, "bb", "Z", &cell_opts(true), Some(b','));
        // 0행: 셀 1(bb)만 → "a,Z,bbc". "bbc"는 부분이라 안 바뀐다.
        // 1행: 셀 0(bb) → "Z,x".
        assert_eq!(total, 2);
        assert_eq!(
            changed,
            vec![(0, "a,Z,bbc".to_string()), (1, "Z,x".to_string())]
        );
    }

    #[test]
    fn replace_whole_cell_sanitizes_newline() {
        // Whole cell replacement의 개행도 공백으로 sanitize(불변식 유지).
        let (s, n) = replace_in_line("a,bb,c", "bb", "x\ny", &cell_opts(true), Some(b','));
        assert_eq!(s, "a,x y,c");
        assert_eq!(n, 1);
        assert!(!s.contains('\n'));
    }

    // ---- unescape ----------------------------------------------------------

    #[test]
    fn unescape_tab() {
        // 이 기능의 존재 이유. `\t` 두 글자가 진짜 탭 한 글자가 된다.
        assert_eq!(unescape(r"a\tb"), "a\tb");
        assert_eq!(unescape(r"\t").chars().count(), 1);
    }

    #[test]
    fn unescape_backslash() {
        // `\\`는 백슬래시 **하나**. 두 글자가 한 글자로 줄어드는지가 핵심이다.
        assert_eq!(unescape(r"a\\b"), r"a\b");
        assert_eq!(unescape(r"C:\\temp"), r"C:\temp");
        // `\\t`는 백슬래시 + t이지 탭이 아니다(백슬래시가 먼저 소비된다).
        assert_eq!(unescape(r"\\t"), r"\t");
    }

    #[test]
    fn unescape_hex() {
        assert_eq!(unescape(r"\x41"), "A");
        assert_eq!(unescape(r"\x09"), "\t");
        assert_eq!(unescape(r"\x7F"), "\u{7F}");
        // 16진수 자릿수는 대소문자를 가리지 않는다.
        assert_eq!(unescape(r"\x4a"), unescape(r"\x4A"));
        assert_eq!(unescape(r"\x4a"), "J");
        // 앞뒤 글자와 섞여도 그 자리만 바뀐다.
        assert_eq!(unescape(r"a\x41b"), "aAb");
    }

    #[test]
    fn unescape_hex_out_of_ascii() {
        // 확정된 해석: `\xNN`은 바이트가 아니라 **유니코드 코드포인트**다.
        // 0x80~0xFF는 U+0080~U+00FF(라틴-1) 문자가 된다 — 한 글자다.
        assert_eq!(unescape(r"\xE9"), "é");
        assert_eq!(unescape(r"\xFF"), "ÿ");
        assert_eq!(unescape(r"\x80"), "\u{80}");
        assert_eq!(unescape(r"\xFF").chars().count(), 1);
        // 유니코드 코드포인트이므로 UTF-8 인코딩은 2바이트다(바이트 해석이라면 1).
        assert_eq!(unescape(r"\xFF").len(), 2);
    }

    #[test]
    fn unescape_leaves_unknown_as_is() {
        // **회귀 못박기**: `\n`은 개행이 되지 않는다. 지원하지 않는 시퀀스는
        // 오류가 아니라 백슬래시 + 그 글자 두 글자로 그대로 남는다.
        assert_eq!(unescape(r"\n"), r"\n");
        assert!(!unescape(r"a\nb").contains('\n'));
        assert_eq!(unescape(r"\r"), r"\r");
        assert_eq!(unescape(r"\z"), r"\z");
        assert_eq!(unescape(r"\0"), r"\0");
        assert_eq!(unescape(r"\u1234"), r"\u1234");
        // 한글 등 비ASCII가 뒤따라도 그대로(char 단위로 흘려보낸다).
        assert_eq!(unescape(r"\가"), r"\가");
    }

    #[test]
    fn unescape_trailing_backslash() {
        assert_eq!(unescape(r"abc\"), r"abc\");
        assert_eq!(unescape(r"\"), r"\");
    }

    #[test]
    fn unescape_bad_hex_left_as_is() {
        // 16진수 2자리가 안 오면 관대 규칙 — 통째로 그대로 남는다.
        assert_eq!(unescape(r"\xZZ"), r"\xZZ");
        assert_eq!(unescape(r"\x9"), r"\x9");
        assert_eq!(unescape(r"\x"), r"\x");
        assert_eq!(unescape(r"\x4"), r"\x4");
        assert_eq!(unescape(r"\xG1"), r"\xG1");
        // 실패해도 뒤 글자를 잡아먹지 않는다.
        assert_eq!(unescape(r"\xZZa"), r"\xZZa");
    }

    #[test]
    fn unescape_hex_reads_exactly_two() {
        // `\x414`는 `\x41`(A) + 남은 `4`.
        assert_eq!(unescape(r"\x414"), "A4");
        assert_eq!(unescape(r"\x4141"), "A41");
    }

    #[test]
    fn unescape_no_backslash_is_identity() {
        // 빠른 경로(memchr)를 타는 입력들. 느린 경로와 같은 결과여야 한다.
        for s in ["", "abc", "a,b\tc", "한글 텍스트", "x\u{7F}y"] {
            assert_eq!(unescape(s), s, "백슬래시가 없으면 입력 그대로");
        }
    }

    #[test]
    fn unescape_fast_path_agrees_with_slow_path() {
        // 빠른 경로가 별도 코드라 결과가 갈릴 수 있다. 백슬래시 없는 입력에
        // 대해 "느린 경로를 강제로 태운 결과"와 같은지 확인한다 — 앞에 `\\`를
        // 붙이면 반드시 느린 경로를 타므로, 그 결과에서 백슬래시 하나를 떼면
        // 빠른 경로 결과와 같아야 한다.
        for s in ["abc", "a,b", "한글", "x\u{7F}y", "tab\there"] {
            let via_slow = unescape(&format!(r"\\{s}"));
            assert_eq!(via_slow, format!(r"\{s}"));
            assert_eq!(&via_slow[1..], unescape(s), "두 경로 결과가 같아야 한다");
        }
    }

    // ---- Task M: 바이트 치환 차등 검증 -------------------------------------
    //
    // `replace_all`의 계약은 "최적화 전과 **비트 단위로** 같은 결과"다. 그래서
    // 옛 경로(전 행에 `replace_in_line`)를 테스트 안에 오라클로 남겨 두고, 새
    // 경로가 그것과 어긋나는 입력이 하나라도 있는지 **전수**로 캔다.
    // (`app.rs`의 `scan_all_matches_differential_fuzz_quote_alphabet`이 같은 규율.)

    /// 최적화 이전의 `replace_all` 그대로 — 빠른 경로도 프리필터도 없이
    /// 전 행에 `replace_in_line`을 돌린다. **정답의 정의**이자 차등 테스트의
    /// 오라클이다. 프로덕션은 이 함수를 부르지 않는다(부르면 최적화가 무의미).
    fn replace_all_reference(
        lines: &[String],
        needle: &str,
        replacement: &str,
        opts: &FindOptions,
        delim: Option<u8>,
    ) -> (Vec<(usize, String)>, usize) {
        if needle.is_empty() {
            return (Vec::new(), 0);
        }
        let mut changed = Vec::new();
        let mut total = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let (new, n) = replace_in_line(line, needle, replacement, opts, delim);
            if n > 0 {
                total += n;
                changed.push((i, new));
            }
        }
        (changed, total)
    }

    /// 한 입력 묶음에 대해 새 경로와 오라클을 비교한다. 어긋나면 어느 입력이
    /// 어떻게 갈렸는지 그대로 보여 준다.
    fn assert_same_as_reference(
        lines: &[String],
        needle: &str,
        rep: &str,
        opts: &FindOptions,
        delim: Option<u8>,
    ) {
        let fast = replace_all(lines, needle, rep, opts, delim);
        let slow = replace_all_reference(lines, needle, rep, opts, delim);
        assert_eq!(
            fast, slow,
            "불일치: lines={lines:?} needle={needle:?} rep={rep:?} \
             match_case={} scope={:?} delim={delim:?}",
            opts.match_case, opts.scope
        );
    }

    /// 길이 `len`까지의 모든 알파벳 문자열을 만든다(빈 문자열 포함).
    fn all_strings(alphabet: &[char], len: usize) -> Vec<String> {
        let mut out = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..len {
            let mut next = Vec::new();
            for s in &frontier {
                for &c in alphabet {
                    let mut t = s.clone();
                    t.push(c);
                    next.push(t);
                }
            }
            out.extend(next.iter().cloned());
            frontier = next;
        }
        out
    }

    /// **핵심 차등 테스트.** 따옴표·구분자·대소문자가 섞인 알파벳으로 만든
    /// **전수** 코퍼스에서 새 경로와 옛 경로가 완전히 같은지.
    ///
    /// 랜덤이 아니라 전수인 이유: Task I에서 랜덤 79.7만 건이 놓친 따옴표
    /// 위음성을 따옴표 전수 프로브가 즉시 잡았다. 알파벳을 작게 잡고 길이를
    /// 늘리는 편이 "이 알파벳 안에서는 반례가 없다"는 강한 진술을 준다.
    #[test]
    fn replace_all_bytes_matches_reference() {
        // `"`(따옴표 함정), `,`(구분자), `a`/`A`(대소문자), `b`(비매치 대조군).
        let alphabet = ['a', 'A', '"', ',', 'b'];
        let corpus = all_strings(&alphabet, 4);
        // 행 하나씩 돌린다(행 간 상호작용이 없는 함수라 조합할 필요가 없고,
        // 4^4까지 전수면 785행 × 아래 needle/rep/옵션 조합이 이미 충분히 크다).
        let batch: Vec<String> = corpus.clone();
        for needle in ["a", "A", "aa", "\"", ",", "a,b", "\"a\"", "ab"] {
            for rep in ["Z", "", "a", "\"", "긴치환문"] {
                for match_case in [true, false] {
                    for scope in [MatchScope::WholeCell, MatchScope::Partial, MatchScope::WholeWord]
                    {
                        let o = FindOptions { match_case, scope };
                        assert_same_as_reference(&batch, needle, rep, &o, Some(b','));
                        // 텍스트 모드(delim=None)도 같은 코퍼스로 — Whole cell이
                        // "행 전체 일치"로 해석되는 가지를 덮는다.
                        assert_same_as_reference(&batch, needle, rep, &o, None);
                    }
                }
            }
        }
    }

    /// 탭 구분자 + 한글 + 빈 셀이 섞인 전수 코퍼스. 사용자 실사용(TSV, 한글
    /// 치환문)과 같은 모양이고, 멀티바이트 문자가 셀 경계 슬라이싱을 깨지
    /// 않는지를 함께 본다.
    #[test]
    fn replace_all_bytes_matches_reference_tsv_hangul() {
        let alphabet = ['-', '\t', '한', 'x', '"'];
        let corpus = all_strings(&alphabet, 4);
        for needle in ["-", "한", "x", "한x", "\t", "\""] {
            for rep in ["한국어", "", "-", "\t"] {
                for match_case in [true, false] {
                    let o = FindOptions { match_case, scope: MatchScope::WholeCell };
                    assert_same_as_reference(&corpus, needle, rep, &o, Some(b'\t'));
                }
            }
        }
    }

    /// 접히는 비ASCII(`É`/`é`)를 알파벳에 넣은 전수 코퍼스. 위 두 코퍼스는
    /// 대소문자가 접히는 문자가 없어 `query_is_case_foldable_by_bytes` 가드를
    /// 지워도 통과해 버린다 — 그 가드를 **차등 테스트가** 지키게 만든다
    /// (전용 단위 테스트 하나에만 의존하면 그 테스트를 지우는 순간 무방비다).
    #[test]
    fn replace_all_bytes_matches_reference_foldable_alphabet() {
        // U+212A(KELVIN SIGN)도 넣는다 — `k`가 든 needle에서 바이트 접기가
        // 유니코드 접기를 놓치는 유일한 통로라, 이 알파벳이 없으면
        // `needle_is_bytefold_exact`의 `k` 배제를 지워도 전수 테스트가 통과한다.
        let alphabet = ['É', 'é', ',', 'a', 'İ', '\u{212A}', 'k'];
        let corpus = all_strings(&alphabet, 3);
        for needle in ["é", "É", "a", "éa", "i\u{307}", "İ", "Éé", "k", "K", "ka", "\u{212A}"] {
            for rep in ["Z", "", "é"] {
                for match_case in [true, false] {
                    for scope in [MatchScope::WholeCell, MatchScope::Partial] {
                        let o = FindOptions { match_case, scope };
                        assert_same_as_reference(&corpus, needle, rep, &o, Some(b','));
                    }
                }
            }
        }
    }

    /// 따옴표가 든 행은 바이트 경로가 `None`으로 물러나고 폴백이 **정확한**
    /// 답을 낸다. 이 함정이 실제로 살아 있는지를 값으로 못박는다 —
    /// `"a"a`는 표시값이 `aa`라 파일 바이트에 `aa`가 없는데도 매치다.
    #[test]
    fn replace_all_quoted_row_falls_back() {
        // 바이트 경로는 이 행에 손대면 안 된다.
        assert_eq!(replace_cells_bytes("\"a\"a,b", "aa", "Z", b',', true), None);
        // 그런데 표시값은 `aa`이므로 매치다 — 폴백이 이것을 잡아야 한다.
        let v = lines(&["\"a\"a,b"]);
        let (changed, total) = replace_all(&v, "aa", "Z", &cell_opts(true), Some(b','));
        assert_eq!(total, 1, "따옴표 셀의 표시값 매치를 놓치면 위음성이다");
        assert_eq!(changed, vec![(0, "Z,b".to_string())]);
        // 따옴표 안의 구분자도 마찬가지(셀 하나다).
        let v = lines(&["\"a,b\",c"]);
        let (changed, total) = replace_all(&v, "a,b", "Z", &cell_opts(true), Some(b','));
        assert_eq!(total, 1);
        assert_eq!(changed, vec![(0, "Z,c".to_string())]);
    }

    /// 한글 needle은 대소문자 개념이 없어 바이트 비교가 곧 정확한 비교다 —
    /// 폴백하지 않고 바이트 경로를 타면서도 정확해야 한다.
    #[test]
    fn replace_all_non_ascii_needle() {
        let v = lines(&["한국\t미국\t한국민", "\t한국\t"]);
        let o = cell_opts(false); // ignore_case
        let (changed, total) = replace_all(&v, "한국", "KR", &o, Some(b'\t'));
        assert_eq!(total, 2, "`한국민`은 셀 부분이라 안 바뀐다");
        assert_eq!(
            changed,
            vec![(0, "KR\t미국\t한국민".to_string()), (1, "\tKR\t".to_string())]
        );
        // 그리고 이 needle은 폴백이 아니라 **바이트 경로**를 탄다.
        assert!(
            replace_cells_bytes("한국\t미국", "한국", "KR", b'\t', false).is_some(),
            "한글 needle은 바이트로 접히므로 폴백할 이유가 없다"
        );
    }

    /// `é`처럼 유니코드 접기가 필요한 needle은 ignore_case에서 바이트 경로가
    /// **물러나야** 한다(`eq_ignore_ascii_case`가 `É`를 못 접어 위음성).
    #[test]
    fn replace_all_foldable_needle_falls_back() {
        assert_eq!(
            replace_cells_bytes("É,b", "é", "Z", b',', false),
            None,
            "ignore_case + 접히는 needle은 바이트로 판정할 수 없다"
        );
        // 폴백이 유니코드 접기로 정확히 잡는다.
        let v = lines(&["É,b"]);
        let (changed, total) = replace_all(&v, "é", "Z", &cell_opts(false), Some(b','));
        assert_eq!(total, 1, "É는 é로 접히므로 매치다");
        assert_eq!(changed, vec![(0, "Z,b".to_string())]);
        // match_case면 접기가 없으므로 바이트 경로를 타도 된다.
        assert!(
            replace_cells_bytes("É,b", "é", "Z", b',', true).is_some(),
            "match_case는 접기가 없어 바이트 비교가 곧 정답이다"
        );
    }

    /// 프리필터 가드(`replace_row_can_skip`)의 단위 테스트. 프로덕션과 이
    /// 테스트가 **같은 함수**를 부른다 — 판정을 루프에 인라인으로 적으면 그것을
    /// 뒤집어도 테스트가 자기 사본만 보고 통과한다.
    ///
    /// 핵심은 **위음성이 없어야 한다**는 것뿐이다: 참(=건너뛰기)은 "매치가 있을
    /// 수 없다"가 증명될 때만.
    #[test]
    fn replace_row_can_skip_never_skips_a_possible_match() {
        let exact = Prefilter::Exact(Box::new(memchr::memmem::Finder::new(b"ab").into_owned()));
        // needle 바이트가 없다 → 건너뛰어도 된다.
        assert!(replace_row_can_skip("xyz,q", &exact, true));
        // 있으면 건너뛰지 않는다.
        assert!(!replace_row_can_skip("xab,q", &exact, true));
        // match_case 프리필터는 대소문자를 구분한다 — `AB`는 needle `ab`의
        // match_case 매치가 아니므로 건너뛰어도 옳다.
        assert!(replace_row_can_skip("AB,q", &exact, true));

        // ignore_case: 대소문자 변형을 전부 살려야 한다(`memmem`이면 놓친다).
        let fold = Prefilter::CaseFold(b"ab".to_vec());
        for hay in ["ab", "AB", "Ab", "aB", "xxAbxx"] {
            assert!(
                !replace_row_can_skip(hay, &fold, true),
                "{hay}는 ignore_case 매치 후보이므로 건너뛰면 위음성이다"
            );
        }
        assert!(replace_row_can_skip("xyz", &fold, true));

        // 따옴표 행은 Whole cell에서 절대 건너뛰지 않는다 — `"a"a`의 표시값은
        // `aa`인데 바이트에는 `aa`가 없다(Task I의 위음성).
        let aa = Prefilter::Exact(Box::new(memchr::memmem::Finder::new(b"aa").into_owned()));
        assert!(
            !replace_row_can_skip("\"a\"a,b", &aa, true),
            "따옴표 행을 바이트로 단정하면 표시값 매치를 잃는다"
        );
        // Partial/WholeWord는 행 원문을 비교하므로 따옴표가 있어도 판정이 정확하다
        // — 여기까지 통과시키면 따옴표 흔한 CSV에서 프리필터가 무력해진다.
        assert!(replace_row_can_skip("\"a\"a,b", &aa, false));

        // Task M 회귀: 홀로 있는 `\r`/`\n`이 낀 행도 quote_sensitive에서 절대
        // 건너뛰지 않는다. needle "aa"가 이 행에 리터럴로 없으므로(바이트에는
        // "aa"가 없다) 이 가드가 없으면 이 프리필터는 건너뛰어도 된다고 판단하는데,
        // `split_fields`(표시값)와 `field_slice`(치환 범위)가 CR/LF를 다르게
        // 취급하는 구조적 위험 자체를 이 가드가 막아야 한다 — 값으로는 아직
        // 위음성을 구성하지 못했더라도, 따옴표와 같은 등급으로 무조건 통과시킨다.
        assert!(
            !replace_row_can_skip("a\r,b", &aa, true),
            "행 안의 CR을 바이트로 단정하면 안 된다(따옴표와 같은 구조적 위험)"
        );
        assert!(
            !replace_row_can_skip("a\n,b", &aa, true),
            "행 안의 LF도 같은 이유로 절대 건너뛰면 안 된다"
        );
        // Partial/WholeWord는 행 원문을 그대로 비교하므로 CR/LF가 있어도
        // 프리필터가 정확하다 — 여기까지 통과시키면 그 행들에서도 프리필터가
        // 무력해진다.
        assert!(replace_row_can_skip("a\r,b", &aa, false));

        // `None`은 판단을 포기한다 — 언제나 통과(안전한 쪽).
        assert!(!replace_row_can_skip("xyz", &Prefilter::None, false));
    }

    /// 바이트 치환이 매치 없는 행에 대해 원문을 **그대로** 돌려주는지.
    /// (프리필터가 통과시킨 뒤에도 셀 단위로는 매치가 없을 수 있다.)
    #[test]
    fn replace_cells_bytes_no_match_returns_original() {
        let (out, n) = replace_cells_bytes("aaa\tbbb\tccc", "aa", "Z", b'\t', true).unwrap();
        assert_eq!((out.as_str(), n), ("aaa\tbbb\tccc", 0), "부분 일치는 셀 전체가 아니다");
        let (out, n) = replace_cells_bytes("aaa\tbbb", "aaa", "Z", b'\t', true).unwrap();
        assert_eq!((out.as_str(), n), ("Z\tbbb", 1));
    }

    /// `needle_is_bytefold_exact`가 `k`/`K` **하나만** 특별 취급해도 되는 근거를
    /// 전 유니코드로 전수 검증한다. 비ASCII인데 소문자화 결과가 **순수 ASCII**인
    /// 문자가 U+212A 하나뿐이어야 한다 — 하나라도 더 생기면(유니코드 데이터는
    /// 버전마다 바뀔 수 있다) 그 문자로 접혀 오는 ASCII 글자도 함께 막아야 하고,
    /// 그 사실을 이 테스트가 **먼저 깨져서** 알려 준다.
    #[test]
    fn only_u212a_lowercases_to_ascii() {
        let mut found: Vec<u32> = Vec::new();
        for cp in 0u32..=0x10FFFF {
            let Some(c) = char::from_u32(cp) else { continue };
            if c.is_ascii() {
                continue;
            }
            let lo: String = c.to_lowercase().collect();
            if !lo.is_empty() && lo.is_ascii() {
                found.push(cp);
            }
        }
        assert_eq!(
            found,
            vec![0x212A],
            "비ASCII → 순수 ASCII로 접히는 문자는 U+212A 하나여야 한다"
        );
        // 그 접힘 결과가 `k`이므로, 막아야 할 ASCII 글자도 `k`/`K` 뿐이다.
        assert_eq!(char::from_u32(0x212A).unwrap().to_lowercase().to_string(), "k");
        assert!(!needle_is_bytefold_exact("k", false));
        assert!(!needle_is_bytefold_exact("King", false));
        assert!(needle_is_bytefold_exact("k", true), "match_case는 접기가 없다");
        assert!(needle_is_bytefold_exact("abc", false));
        assert!(needle_is_bytefold_exact("한국어", false));
        assert!(!needle_is_bytefold_exact("é", false));
    }

    /// **U+212A KELVIN SIGN(`K`)** — 비ASCII인데 소문자화하면 ASCII `k`가 되는
    /// 유니코드 유일의 문자다. 그래서 ignore_case + ASCII needle `k`에서
    /// 바이트 접기(`find_ci_ascii`)와 유니코드 접기(`find_in_line`)의 답이 갈린다.
    ///
    /// 스캔 경로에는 이 어긋남이 **의도적으로 남아 있지만**(`app.rs`의
    /// `query_is_case_foldable_by_bytes` 주석 — 흔한 글자 `k` 때문에 빠른 경로를
    /// 통째로 잃는 대가가 더 크다), 치환의 계약은 "최적화 전과 비트 단위로 동일"이라
    /// 여기서는 **어긋나면 안 된다**. 이 테스트가 그 사실을 값으로 못박는다.
    #[test]
    fn replace_all_kelvin_sign_matches_reference() {
        let v = lines(&["\u{212A}", "\u{212A},x", "k,\u{212A}", "K"]);
        for scope in [MatchScope::WholeCell, MatchScope::Partial, MatchScope::WholeWord] {
            let o = FindOptions { match_case: false, scope };
            assert_same_as_reference(&v, "k", "Z", &o, Some(b','));
            assert_same_as_reference(&v, "K", "Z", &o, Some(b','));
            assert_same_as_reference(&v, "k", "Z", &o, None);
        }
    }

    /// 치환문의 개행 방어가 바이트 경로에서도 살아 있다 — `replace_all`이
    /// `sanitize_for_line`을 **한 번만** 접어 두고 바이트 경로에 넘기므로,
    /// 그 배선이 끊기면 `lines[i]`에 개행이 박혀 "한 줄 = 한 행"이 깨진다.
    #[test]
    fn replace_all_bytes_sanitizes_newline_in_replacement() {
        let v = lines(&["a\tb"]);
        let (changed, total) = replace_all(&v, "a", "x\ny", &cell_opts(true), Some(b'\t'));
        assert_eq!(total, 1);
        assert_eq!(changed, vec![(0, "x y\tb".to_string())]);
        assert!(!changed[0].1.contains('\n'));
    }

    // ── Task M 회귀: 행 안의 홀로 있는 `\r` ─────────────────────────────
    //
    // `csv_core`는 `\r`/`\n`을 **레코드 종결자**로 취급하므로
    // `split_fields("a\r,b", b',')`는 셀 하나("a")만 준다. 하지만
    // `replace_cells_bytes`는 델리미터만 보고 그대로 둘로 쪼갠다
    // (`["a\r", "b"]`) — csv_core 의미와 바이트 분할이 여기서 갈린다.
    // 이 갈림이 실제로 파일 로드 경로(`load_edit_buffer`)를 통해 `lines[i]`에
    // 도달할 수 있다(옛 Mac 개행이나 내보낸 데이터의 임베디드 CR).
    // `replace_cells_bytes`도 `replace_row_can_skip`도 이 셀 분할 방식을
    // 그대로 물려받으므로, `\r`을 quote와 같은 등급의 가드로 막아야 한다.

    /// 리뷰어가 제시한 재현 1: `needle="a"`가 새 경로에서 위음성으로 사라진다.
    /// old(오라클, `replace_in_line`)는 `split_fields("a\r,b", ',')`가 셀 하나
    /// `"a"`만 주므로 그 셀이 needle "a"와 정확히 같아 매치다 — 실제로 **1건
    /// 바꾼다**. 새 경로(수정 전)는 델리미터만 보고 셀을 `"a\r"`/`"b"` 둘로
    /// 쪼개 `"a\r"` != `"a"`라 판정해 놓친다. old의 행동값 자체를 오라클로
    /// 못박는다(csv_core 내부 동작을 재해석하지 않는다).
    #[test]
    fn replace_all_bare_cr_regression_needle_a() {
        let bytes = b"h1,h2\na\r,b\nc,d\n".to_vec();
        let source = Source::from_bytes(bytes);
        let buf = load_edit_buffer(&source, Encoding::Utf8);
        assert_eq!(buf.lines, vec!["h1,h2", "a\r,b", "c,d"], "CR이 그대로 살아 lines[1]에 남아야 한다");

        let o = cell_opts(true); // match_case
        let fast = replace_all(&buf.lines, "a", "Z", &o, Some(b','));
        let slow = replace_all_reference(&buf.lines, "a", "Z", &o, Some(b','));
        assert_eq!(fast, slow, "바이트 경로가 CR을 못 보고 오라클과 갈리면 안 된다");
        assert_eq!(slow, (vec![(1, "Z,b".to_string())], 1), "old 오라클 값 고정");
    }

    /// 리뷰어가 제시한 재현 2: `needle="b"`가 위양성으로 나타난다(같은 행).
    /// old는 셀을 `a\r`/`b` 둘로 나누지 않으므로 `b`만 있는 셀이 없어 매치가
    /// 없다. 새 경로가 델리미터만 보고 쪼개면 `b` 셀이 생겨 잘못 바뀐다.
    #[test]
    fn replace_all_bare_cr_regression_needle_b() {
        let bytes = b"h1,h2\na\r,b\nc,d\n".to_vec();
        let source = Source::from_bytes(bytes);
        let buf = load_edit_buffer(&source, Encoding::Utf8);

        let o = cell_opts(true);
        let fast = replace_all(&buf.lines, "b", "Z", &o, Some(b','));
        let slow = replace_all_reference(&buf.lines, "b", "Z", &o, Some(b','));
        assert_eq!(fast, slow, "바이트 경로가 CR 행에 없는 셀 경계를 만들어내면 안 된다");
        assert_eq!(slow, (Vec::new(), 0), "old 오라클: 매치 없음");
    }

    /// 리뷰어가 제시한 재현 3: 실사용 형태인 TSV + 한글, `-\r\t한국`에서
    /// needle `한국`. old는 `-\r`을 한 셀로 보아 `한국`만 있는 셀이 없다.
    #[test]
    fn replace_all_bare_cr_regression_tsv_hangul() {
        let v = lines(&["-\r\t한국"]);
        let o = cell_opts(true);
        let fast = replace_all(&v, "한국", "X", &o, Some(b'\t'));
        let slow = replace_all_reference(&v, "한국", "X", &o, Some(b'\t'));
        assert_eq!(fast, slow, "TSV + 한글 + CR 조합에서 바이트 경로가 오라클과 갈리면 안 된다");
        assert_eq!(slow, (Vec::new(), 0), "old 오라클: 매치 없음");
    }

    /// **핵심 차등 테스트에 `\r`/`\n`을 더한 버전.** 기존 세 코퍼스의 알파벳
    /// (`{a,A,",,,b}`, `{-,\t,한,x,"}`, `{É,é,,,a,İ,U+212A,k}`)은 전부 csv_core가
    /// 레코드 종결자로 특별 취급하는 제어 바이트를 담지 않아, 이 버그가 있어도
    /// 전수 테스트를 통과했다. 이 코퍼스가 그 구멍을 영구히 메운다 — 수정 전엔
    /// 반드시 실패하고 수정 후엔 통과해야 한다.
    #[test]
    fn replace_all_bytes_matches_reference_cr_lf_alphabet() {
        let alphabet = ['a', ',', '\r', '\n', 'b'];
        let corpus = all_strings(&alphabet, 4);
        for needle in ["a", "b", "a,b", "\r", "\n", "a\r"] {
            for rep in ["Z", "", "a"] {
                for match_case in [true, false] {
                    for scope in [MatchScope::WholeCell, MatchScope::Partial, MatchScope::WholeWord]
                    {
                        let o = FindOptions { match_case, scope };
                        assert_same_as_reference(&corpus, needle, rep, &o, Some(b','));
                    }
                }
            }
        }
    }
}
