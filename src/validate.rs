//! 파싱 오류 행 검출.
//!
//! 표 모드에서 "전체 규칙(= 기대 컬럼 수)과 어긋나는 행"을 찾아 목록으로
//! 돌려준다. 순수 로직만 있고 egui에 의존하지 않는다(`app.rs`가 배선한다).

use crate::index::LineIndex;
use crate::parse::{decode_line, split_fields, Encoding};
use crate::source::Source;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 한 행이 전체 규칙과 어긋나는 유형.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIssue {
    /// 필드 수가 기대치와 다름.
    FieldCount { got: usize, expected: usize },
    /// 큰따옴표가 열리고 닫히지 않음(개수 홀수).
    UnbalancedQuote,
    /// 현재 인코딩으로 디코딩 실패(대체문자 U+FFFD 발생).
    DecodeError,
}

/// 오류가 발견된 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    /// 논리 행번호(헤더 포함 좌표계, 0-based).
    pub logical: usize,
    pub issue: RowIssue,
    /// 목록 표시용 줄 앞부분(최대 `PREVIEW_CHARS`자).
    pub preview: String,
}

/// 오류 창 목록에 보여줄 줄 미리보기 최대 길이(문자).
pub const PREVIEW_CHARS: usize = 120;

/// 한 줄을 검사해 첫 번째로 발견된 문제를 돌려준다. 문제 없으면 `None`.
///
/// 검사 순서: 디코드 실패 → 따옴표 불균형 → 필드 수. 이 순서인 이유는 **뒤의
/// 검사가 앞의 실패에 오염되기 때문**이다 — 디코딩이 깨진 줄은 구분자 바이트가
/// 대체문자로 바뀌어 필드 수가 엉뚱하게 나오고, 따옴표가 안 닫힌 줄은
/// `split_fields`가 남은 전부를 한 필드로 삼아 역시 필드 수가 어긋난다.
/// 그때 `FieldCount`를 보고하면 사용자가 진짜 원인 대신 증상을 좇게 된다.
///
/// 빈 줄은 오류로 보지 않는다(파일 끝 개행·구분용 빈 줄이 흔하다).
pub fn check_line(line: &str, delim: u8, expected_cols: usize) -> Option<RowIssue> {
    if line.is_empty() {
        return None;
    }
    // 1) 디코드 실패(대체문자).
    if line.contains('\u{FFFD}') {
        return Some(RowIssue::DecodeError);
    }
    // 2) 따옴표 불균형(개수 홀수). `"`는 ASCII라 바이트로 세도 UTF-8
    //    멀티바이트 문자의 내부 바이트와 겹치지 않는다(선행/후속 바이트는
    //    모두 0x80 이상).
    if line.bytes().filter(|&b| b == b'"').count() % 2 != 0 {
        return Some(RowIssue::UnbalancedQuote);
    }
    // 3) 필드 수 불일치.
    let got = split_fields(line, delim).len();
    if got != expected_cols {
        return Some(RowIssue::FieldCount { got, expected: expected_cols });
    }
    None
}

/// 줄에서 목록 표시용 미리보기를 만든다(최대 `PREVIEW_CHARS`자, 넘으면 `…` 부착).
///
/// 바이트가 아니라 **문자** 단위로 자른다 — 바이트로 자르면 한글 중간에서
/// 끊겨 `String`이 UTF-8 경계를 어겨 패닉한다.
pub fn preview_of(line: &str) -> String {
    let mut out: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

/// 스캔 한 번의 결과. 수집한 오류와, 상한에 걸려 **버려진** 개수.
///
/// 버린 개수를 함께 돌려주는 이유: 상한만 두고 조용히 자르면 "오류 10,000개"가
/// 화면에 뜨는데 실제로는 300만 개일 수 있다. 사용자가 목록을 다 훑고 "다
/// 고쳤다"고 믿게 되는 것이 가장 나쁜 결과다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub errors: Vec<RowError>,
    /// 상한 초과로 목록에 담지 못한 오류 수.
    pub dropped: usize,
}

impl ScanResult {
    /// 발견된 오류 총계(수집분 + 버려진 분).
    pub fn total(&self) -> usize {
        self.errors.len() + self.dropped
    }

    /// 유형별 개수 — (필드 수, 따옴표, 디코드). **수집된 것만** 센다.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut fc = 0;
        let mut uq = 0;
        let mut de = 0;
        for e in &self.errors {
            match e.issue {
                RowIssue::FieldCount { .. } => fc += 1,
                RowIssue::UnbalancedQuote => uq += 1,
                RowIssue::DecodeError => de += 1,
            }
        }
        (fc, uq, de)
    }
}

/// 두 부분 결과를 합쳐 **논리 행번호가 작은 쪽 `limit`개**만 남긴다(rayon reduce용).
///
/// 상한을 "먼저 도착한 것 우선"이 아니라 **행번호 우선**으로 정한 이유:
/// rayon의 병합 순서는 API가 보장하지 않는다. 도착 순서에 기대면 같은 파일을
/// 두 번 열어도 목록이 달라질 수 있고, 무엇보다 사용자가 원하는 것은
/// "파일 앞쪽부터 고쳐 나가는 것"이지 "스레드가 먼저 끝낸 조각"이 아니다.
///
/// **전제:** `a`와 `b`는 각각 행번호 오름차순이다(청크가 행을 오름차순으로
/// 훑고, 이 함수가 그 성질을 보존한다 — 귀납). 그래서 정렬이 아니라 **병합**
/// 이면 충분하고, 결과도 오름차순이다.
///
/// 버려진 개수(`dropped`)는 그대로 센다 — 상한만 두고 조용히 자르면 사용자가
/// 목록을 다 훑고 "다 고쳤다"고 믿게 되는 것이 가장 나쁜 결과다.
fn merge_results(a: ScanResult, b: ScanResult, limit: usize) -> ScanResult {
    debug_assert!(a.errors.windows(2).all(|w| w[0].logical <= w[1].logical));
    debug_assert!(b.errors.windows(2).all(|w| w[0].logical <= w[1].logical));

    let total = a.errors.len() + b.errors.len();
    let mut errors = Vec::with_capacity(total.min(limit));
    let (mut ia, mut ib) = (a.errors.into_iter().peekable(), b.errors.into_iter().peekable());
    while errors.len() < limit {
        let take_a = match (ia.peek(), ib.peek()) {
            (Some(x), Some(y)) => x.logical <= y.logical,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        errors.push(if take_a { ia.next() } else { ib.next() }.expect("peek이 Some"));
    }
    ScanResult {
        dropped: a.dropped + b.dropped + (total - errors.len()),
        errors,
    }
}

/// 병렬 청크 크기. 행 하나당 하는 일이 작아서(대개 `split_fields` 한 번)
/// 청크가 작으면 rayon 작업 분배 비용이 검사 자체보다 커진다.
const CHUNK: usize = 64 * 1024;

/// 청크 안에서 취소 신호를 몇 행마다 확인할지. 청크 경계에서만 보면 이미
/// 시작한 청크가 `CHUNK`행을 다 돌 때까지 멈추지 않는다.
const CANCEL_CHECK_ROWS: usize = 1024;

/// 이미 디코딩된 줄들(편집 버퍼)을 병렬 검사한다.
///
/// mmap 경로와 나누어 둔 이유: **편집 모드에서는 파일 바이트가 더 이상 진실이
/// 아니다.** 셀을 고치거나 구분자를 변환한 뒤에도 mmap을 스캔하면 디스크에
/// 남은 옛 내용을 검사하게 되어, 사용자가 방금 고친 행이 계속 오류로 남고
/// 방금 망가뜨린 행은 잡히지 않는다.
pub fn scan_lines(
    lines: &[String],
    delim: u8,
    expected_cols: usize,
    data_start: usize,
    limit: usize,
) -> ScanResult {
    if lines.len() <= data_start {
        return ScanResult::default();
    }
    (data_start..lines.len())
        .into_par_iter()
        .chunks(CHUNK)
        .map(|chunk| {
            let mut local = ScanResult::default();
            for logical in chunk {
                let line = &lines[logical];
                if let Some(issue) = check_line(line, delim, expected_cols) {
                    // 청크 안에서는 상한을 걸지 않는다 — 전역 병합에서 센다.
                    local.errors.push(RowError {
                        logical,
                        issue,
                        preview: preview_of(line),
                    });
                }
            }
            local
        })
        .reduce(ScanResult::default, |a, b| merge_results(a, b, limit))
}

/// 데이터 행 전체를 mmap에서 병렬 스캔해 규칙에 어긋난 행을 모은다(뷰 모드).
///
/// - `expected_cols`: 기대 컬럼 수(헤더 필드 수 또는 앞 데이터 행 샘플 최댓값).
/// - `data_start`: 검사 시작 논리 행(헤더 있으면 1).
/// - `limit`: 수집 상한(초과분은 `dropped`로 센다).
/// - `cancel`: 참을 돌려주면 즉시 중단하고 그때까지 모은 것을 돌려준다.
///   대용량 파일에서 탭을 닫거나 구분자를 바꿨을 때 죽은 스캔이 코어를 붙들고
///   있지 않도록 한다.
///
/// 결과는 논리 행번호 오름차순.
#[allow(clippy::too_many_arguments)]
pub fn scan_errors(
    source: &Arc<Source>,
    index: &LineIndex,
    enc: Encoding,
    delim: u8,
    expected_cols: usize,
    data_start: usize,
    limit: usize,
    cancel: Option<&(dyn Fn() -> bool + Sync)>,
) -> ScanResult {
    let total = index.line_count();
    if total <= data_start {
        return ScanResult::default();
    }
    let (offsets, total_bytes) = index.snapshot();
    let offsets: &[u64] = &offsets;
    let bytes = source.as_bytes();

    (data_start..total)
        .into_par_iter()
        .chunks(CHUNK)
        .map(|chunk| {
            let mut local = ScanResult::default();
            if cancel.is_some_and(|c| c()) {
                return local;
            }
            for (i, logical) in chunk.into_iter().enumerate() {
                // 청크 진입에서 한 번만 보면 이미 시작한 청크가 6.5만 행을 전부
                // 디코딩할 때까지 코어를 붙든다. 취소는 "탭을 닫았다"처럼 결과가
                // 이미 버려진 상황이라, 그만큼을 마저 도는 것은 순수한 낭비다.
                //
                // 원자적 읽기 하나가 1024행마다 한 번이면 스캔 비용에 묻힌다
                // (행마다 보면 그 읽기가 검사 자체보다 비싸진다).
                if i % CANCEL_CHECK_ROWS == 0 && i > 0 && cancel.is_some_and(|c| c()) {
                    return local;
                }
                let Some((s, e)) = LineIndex::range_in(offsets, total_bytes, logical) else {
                    continue;
                };
                let raw = &bytes[s as usize..e as usize];
                let text = decode_line(raw, enc);
                // 인덱스의 행 범위는 개행을 **포함**한다. 떼지 않으면 마지막
                // 필드에 `\n`이 붙어 값이 달라 보이고, CRLF 파일에서는 `\r`이
                // 미리보기 끝에 남는다.
                let line = text.trim_end_matches(['\r', '\n']);
                if let Some(issue) = check_line(line, delim, expected_cols) {
                    local.errors.push(RowError {
                        logical,
                        issue,
                        preview: preview_of(line),
                    });
                }
            }
            local
        })
        .reduce(ScanResult::default, |a, b| merge_results(a, b, limit))
}

/// 오류 목록에 담을 최대 행 수. 초과분은 `ScanResult::dropped`로 센다.
///
/// 상한이 필요한 이유: 구분자를 잘못 고르면 **모든 행**이 오류가 된다
/// (1,500만 행짜리 파일에서 `RowError` 1,500만 개 = 수 GB). 목록을 다 담아도
/// 사람이 볼 수 있는 양이 아니다.
pub const MAX_ROW_ERRORS: usize = 10_000;

/// 백그라운드 오류 검사 작업. `sort::SortJob`과 같은 규율이다 — UI를 막지
/// 않고, 폴링으로 결과를 수거하며, 취소할 수 있다.
pub struct ScanJob {
    result: Arc<Mutex<Option<ScanResult>>>,
    finished: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ScanJob {
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    /// 완료됐으면 결과를 꺼낸다(한 번만). 미완료면 `None`.
    pub fn take_result(&mut self) -> Option<ScanResult> {
        if !self.is_finished() {
            return None;
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.result.lock().unwrap().take()
    }

    /// 취소를 요청한다. 스레드는 다음 청크 경계에서 멈춘다.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 스레드를 join하지 않고 버려도 프로세스가 종료될 때까지 코어를 붙들지
/// 않도록, 드롭 시 취소를 신호한다. join까지 기다리지는 않는다 — UI 스레드가
/// 탭을 닫는 순간 멈추면 안 되기 때문이다.
impl Drop for ScanJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// mmap을 백그라운드에서 스캔한다. 인덱싱이 끝난 뒤에 부를 것.
#[allow(clippy::too_many_arguments)]
pub fn spawn_scan(
    source: Arc<Source>,
    index: LineIndex,
    enc: Encoding,
    delim: u8,
    expected_cols: usize,
    data_start: usize,
    ctx: egui::Context,
) -> ScanJob {
    let result = Arc::new(Mutex::new(None));
    let finished = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));

    let (result_bg, finished_bg, cancel_bg) = (result.clone(), finished.clone(), cancel.clone());
    let handle = std::thread::spawn(move || {
        let is_cancelled = {
            let cancel = cancel_bg.clone();
            move || cancel.load(Ordering::Relaxed)
        };
        let r = scan_errors(
            &source,
            &index,
            enc,
            delim,
            expected_cols,
            data_start,
            MAX_ROW_ERRORS,
            Some(&is_cancelled),
        );
        // 취소된 결과는 부분적이라 버린다 — 반쪽짜리 목록을 "검사 완료"로
        // 보여 주면 사용자가 남은 오류를 없는 것으로 오해한다.
        if !cancel_bg.load(Ordering::Relaxed) {
            *result_bg.lock().unwrap() = Some(r);
        }
        finished_bg.store(true, Ordering::Relaxed);
        ctx.request_repaint();
    });

    ScanJob {
        result,
        finished,
        cancel,
        handle: Some(handle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_line_has_no_issue() {
        assert_eq!(check_line("a,b,c", b',', 3), None);
    }

    #[test]
    fn field_count_too_few() {
        assert_eq!(
            check_line("a,b", b',', 3),
            Some(RowIssue::FieldCount { got: 2, expected: 3 })
        );
    }

    #[test]
    fn field_count_too_many() {
        assert_eq!(
            check_line("a,b,c,d", b',', 3),
            Some(RowIssue::FieldCount { got: 4, expected: 3 })
        );
    }

    #[test]
    fn unbalanced_quote_detected() {
        // 따옴표가 홀수 개 → 열리고 안 닫힘.
        assert_eq!(
            check_line("a,\"b,c", b',', 3),
            Some(RowIssue::UnbalancedQuote)
        );
    }

    #[test]
    fn balanced_quote_is_ok() {
        // "b,c" 는 한 필드 → 총 2필드. expected 2면 정상.
        assert_eq!(check_line("a,\"b,c\"", b',', 2), None);
    }

    #[test]
    fn decode_error_detected() {
        let line = "a,\u{FFFD},c";
        assert_eq!(check_line(line, b',', 3), Some(RowIssue::DecodeError));
    }

    #[test]
    fn empty_line_is_not_an_error() {
        assert_eq!(check_line("", b',', 3), None);
    }

    /// 검사 순서가 규율대로인지 — 세 문제가 **동시에** 있는 줄은 가장 근본
    /// 원인(디코드)을 보고해야 한다. 순서를 뒤집으면 이 테스트가 실패한다.
    #[test]
    fn decode_error_wins_over_other_issues() {
        // 대체문자 + 홀수 따옴표 + 필드 수 불일치를 한 줄에 담는다.
        let line = "\u{FFFD},\"b";
        assert_eq!(check_line(line, b',', 5), Some(RowIssue::DecodeError));
    }

    /// 따옴표가 안 닫히면 `split_fields`가 남은 전부를 한 필드로 삼아 필드
    /// 수도 어긋난다. 그때 `FieldCount`가 아니라 `UnbalancedQuote`를 보고해야
    /// 사용자가 진짜 원인을 본다.
    #[test]
    fn unbalanced_quote_wins_over_field_count() {
        let line = "a,\"b,c";
        // 필드 수로도 걸리는 상황인지 먼저 확인(테스트의 전제).
        assert_ne!(split_fields(line, b',').len(), 3);
        assert_eq!(
            check_line(line, b',', 3),
            Some(RowIssue::UnbalancedQuote)
        );
    }

    /// 이스케이프된 따옴표(`""`)는 짝수이므로 불균형이 아니다.
    #[test]
    fn escaped_quotes_are_balanced() {
        // "a""b" → 한 필드, 따옴표 4개.
        assert_eq!(check_line("\"a\"\"b\",c", b',', 2), None);
    }

    #[test]
    fn preview_truncates_long_line() {
        let long: String = "x".repeat(300);
        let p = preview_of(&long);
        assert!(p.chars().count() <= PREVIEW_CHARS + 1); // + '…'
        assert!(p.ends_with('…'));
    }

    #[test]
    fn preview_keeps_short_line() {
        assert_eq!(preview_of("abc"), "abc");
    }

    /// 미리보기를 **바이트**로 자르면 한글 중간에서 끊겨 패닉한다.
    /// 문자 단위로 자르는지 확인한다.
    #[test]
    fn preview_cuts_on_char_boundary_for_multibyte() {
        let long: String = "한".repeat(300);
        let p = preview_of(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.starts_with('한'));
        assert!(p.ends_with('…'));
    }

    // ---- merge_results (상한·순서 규칙) ----
    //
    // 병합을 **직접** 검사한다. `scan_lines` 너머로만 보면 rayon이 청크를
    // 왼쪽부터 순서대로 접어 주는 바람에 "행번호가 큰 쪽이 먼저 온" 상황이
    // 아예 만들어지지 않아, 병합 규칙을 지워도 테스트가 통과한다(변이
    // 테스트에서 실제로 그랬다). 여기서는 그 상황을 손으로 구성한다.

    fn res(rows: &[usize], dropped: usize) -> ScanResult {
        ScanResult {
            errors: rows
                .iter()
                .map(|&logical| RowError {
                    logical,
                    issue: RowIssue::FieldCount { got: 1, expected: 2 },
                    preview: format!("row{logical}"),
                })
                .collect(),
            dropped,
        }
    }

    fn rows_of(r: &ScanResult) -> Vec<usize> {
        r.errors.iter().map(|e| e.logical).collect()
    }

    /// 오른쪽이 더 작은 행번호를 갖고 있어도 결과는 오름차순이어야 한다.
    #[test]
    fn merge_interleaves_in_row_order() {
        let m = merge_results(res(&[10, 30, 50], 0), res(&[20, 40], 0), 100);
        assert_eq!(rows_of(&m), vec![10, 20, 30, 40, 50]);
        assert_eq!(m.dropped, 0);
    }

    /// 상한에 걸리면 **행번호가 작은 쪽**이 남는다 — 어느 인자에 있었든.
    #[test]
    fn merge_limit_keeps_smallest_rows_regardless_of_side() {
        let m = merge_results(res(&[100, 200], 0), res(&[1, 2], 0), 3);
        assert_eq!(rows_of(&m), vec![1, 2, 100], "작은 쪽 3개");
        assert_eq!(m.dropped, 1, "잘린 1개를 센다");
    }

    /// 양쪽이 이미 갖고 있던 `dropped`는 보존된다(누적).
    #[test]
    fn merge_accumulates_dropped_from_both_sides() {
        let m = merge_results(res(&[1], 5), res(&[2], 7), 100);
        assert_eq!(rows_of(&m), vec![1, 2]);
        assert_eq!(m.dropped, 12);
        assert_eq!(m.total(), 14);
    }

    /// limit이 0이면 아무것도 수집하지 않고 전부 버린 것으로 센다.
    #[test]
    fn merge_limit_zero_drops_everything() {
        let m = merge_results(res(&[1, 2], 0), res(&[3], 0), 0);
        assert!(m.errors.is_empty());
        assert_eq!(m.dropped, 3);
    }

    /// 빈 쪽과의 병합은 항등이다 — **단, 상한을 이미 지키는 값에 한해서.**
    ///
    /// 정확히 적는 이유: `merge_results(x, default(), limit)`은 `x`가 상한을
    /// 넘으면 잘라 낸다. 그러니 "항상 항등"이라고 쓰면 거짓인 법을 주장하는
    /// 셈이고, 그 테스트는 상한 로직이 통째로 없는 구현도 통과시킨다
    /// (`limit`을 입력보다 크게 잡으면 자를 일이 없으므로).
    ///
    /// rayon이 실제로 요구하는 것은 이 **제한된** 항등이다. reduce에 흘러
    /// 다니는 값은 이미 병합을 거쳐 상한을 지키고 있기 때문이다.
    #[test]
    fn merge_with_empty_is_identity_for_limit_respecting_values() {
        let a = res(&[3, 9], 2);
        assert_eq!(merge_results(a.clone(), ScanResult::default(), 100), a);
        assert_eq!(merge_results(ScanResult::default(), a.clone(), 100), a);
        // 상한과 정확히 같은 크기(경계)에서도 항등이다.
        assert_eq!(merge_results(a.clone(), ScanResult::default(), 2), a);
    }

    /// 상한을 **넘는** 값은 빈 쪽과 병합해도 그대로가 아니다 — 잘린다.
    /// 이것이 의도된 동작임을 못 박아, 위 테스트가 "항상 항등"으로 잘못
    /// 넓어지는 것을 막는다.
    #[test]
    fn merge_with_empty_truncates_over_limit_values() {
        let a = res(&[1, 2, 3, 4], 0);
        let m = merge_results(a, ScanResult::default(), 2);
        assert_eq!(rows_of(&m), vec![1, 2]);
        assert_eq!(m.dropped, 2, "잘린 만큼 센다");
    }

    /// **결합법칙 + 임의 폴드 트리**에서 결과가 같아야 한다.
    ///
    /// rayon이 범위를 어떻게 쪼개고 어떤 순서로 접는지는 스레드 수에 따라
    /// 달라진다. 그래서 "특정 분할에서 맞다"로는 부족하고, 가능한 분할 전반에서
    /// 같은 답이 나와야 한다. 여기서는 rayon의 실제 의미(각 조각을
    /// `default()`로 시드해 순차 폴드한 뒤, 그 결과들을 임의의 이진 트리로
    /// 병합)를 흉내 내 전수에 가깝게 확인한다.
    ///
    /// 기준 정답은 "행번호가 가장 작은 limit개, 나머지는 dropped".
    #[test]
    fn merge_is_associative_across_fold_trees() {
        // 행번호 12개를 오름차순으로 두고, 여러 방식으로 쪼개 접는다.
        let all: Vec<usize> = (1..=12).collect();

        // rayon 한 조각의 순차 폴드: default()로 시드해 원소를 하나씩 접는다.
        fn fold_chunk(rows: &[usize], limit: usize) -> ScanResult {
            let mut acc = ScanResult::default();
            for &r in rows {
                acc = merge_results(acc, res(&[r], 0), limit);
            }
            acc
        }

        // 조각 경계를 비트마스크로 전수 생성(11개 경계 → 2048가지).
        for mask in 0u32..(1 << 11) {
            let mut chunks: Vec<Vec<usize>> = Vec::new();
            let mut cur: Vec<usize> = vec![all[0]];
            for (i, &row) in all.iter().enumerate().skip(1) {
                if mask & (1 << (i - 1)) != 0 {
                    chunks.push(std::mem::take(&mut cur));
                }
                cur.push(row);
            }
            chunks.push(cur);

            for &limit in &[0usize, 1, 5, 12, 100] {
                let folded: Vec<ScanResult> =
                    chunks.iter().map(|c| fold_chunk(c, limit)).collect();

                // 왼쪽 결합으로 접기.
                let left = folded
                    .iter()
                    .cloned()
                    .fold(ScanResult::default(), |a, b| merge_results(a, b, limit));
                // 오른쪽 결합으로 접기(트리 모양이 달라도 같아야 한다).
                let right = folded
                    .iter()
                    .cloned()
                    .rev()
                    .fold(ScanResult::default(), |a, b| merge_results(b, a, limit));

                let expect_kept = all.len().min(limit);
                assert_eq!(
                    rows_of(&left),
                    all[..expect_kept],
                    "mask={mask} limit={limit}"
                );
                assert_eq!(left, right, "폴드 트리 모양에 결과가 좌우된다 mask={mask}");
                assert_eq!(
                    left.total(),
                    all.len(),
                    "총계는 분할·상한과 무관해야 한다 mask={mask} limit={limit}"
                );
            }
        }
    }

    /// 동점(같은 행번호)이 있어도 개수가 맞아야 한다 — 한쪽을 통째로 버리는
    /// 구현이면 여기서 걸린다.
    #[test]
    fn merge_keeps_duplicates_at_same_row() {
        let m = merge_results(res(&[7], 0), res(&[7], 0), 100);
        assert_eq!(rows_of(&m), vec![7, 7]);
    }

    // ---- scan_lines (편집 버퍼 경로) ----

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scan_lines_finds_bad_rows_with_logical_numbers() {
        let src = lines(&["a,b,c", "1,2,3", "4,5", "6,7,8"]);
        let r = scan_lines(&src, b',', 3, 1, 100);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].logical, 2);
        assert_eq!(
            r.errors[0].issue,
            RowIssue::FieldCount { got: 2, expected: 3 }
        );
        assert_eq!(r.errors[0].preview, "4,5");
        assert_eq!(r.dropped, 0);
    }

    #[test]
    fn scan_lines_respects_data_start_skipping_header() {
        // 헤더 자체의 필드 수가 달라도 data_start=1이면 검사 대상이 아니다.
        let src = lines(&["only_one_col", "1,2,3"]);
        let r = scan_lines(&src, b',', 3, 1, 100);
        assert!(r.errors.is_empty(), "헤더 행은 검사에서 제외");
    }

    #[test]
    fn scan_lines_without_header_checks_row_zero() {
        // data_start=0이면 0번 행도 검사 대상이다.
        let src = lines(&["only_one_col", "1,2,3"]);
        let r = scan_lines(&src, b',', 3, 0, 100);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].logical, 0);
    }

    #[test]
    fn scan_lines_results_sorted_by_logical() {
        let src = lines(&["a,b", "1", "2,3", "4", "5"]);
        let r = scan_lines(&src, b',', 2, 1, 100);
        let nums: Vec<usize> = r.errors.iter().map(|e| e.logical).collect();
        assert_eq!(nums, vec![1, 3, 4]);
    }

    #[test]
    fn scan_lines_limit_caps_collected_and_counts_dropped() {
        let src = lines(&["a,b", "1", "2", "3"]);
        let r = scan_lines(&src, b',', 2, 1, 2);
        assert_eq!(r.errors.len(), 2, "수집은 상한까지만");
        assert_eq!(r.dropped, 1, "버린 개수를 보고한다");
        assert_eq!(r.total(), 3, "총계는 실제 오류 수");
    }

    /// 상한에 걸렸을 때 **어떤** 오류가 남는지가 결정적이어야 한다.
    ///
    /// 오류를 여러 청크에 흩뿌려(청크 경계를 여러 번 넘게) 병렬 병합을 실제로
    /// 여러 번 돌린 뒤, 남은 것이 정확히 **행번호가 가장 작은 limit개**인지
    /// 본다. 도착 순서에 기대는 구현이면 이 단언이 깨진다.
    #[test]
    fn scan_lines_limit_keeps_lowest_row_numbers() {
        const ROWS: usize = CHUNK * 6;
        const STRIDE: usize = 9_973; // 청크 크기와 서로소인 소수 간격
        let mut src = vec!["a,b".to_string()];
        let mut bad_rows: Vec<usize> = Vec::new();
        for i in 0..ROWS {
            let logical = i + 1; // 0번은 헤더
            if i % STRIDE == 0 {
                src.push("bad".to_string()); // 필드 1개 → FieldCount
                bad_rows.push(logical);
            } else {
                src.push("ok,ok".to_string());
            }
        }
        let limit = bad_rows.len() / 2;
        let r = scan_lines(&src, b',', 2, 1, limit);

        assert_eq!(r.errors.len(), limit);
        assert_eq!(r.dropped, bad_rows.len() - limit);
        assert_eq!(r.total(), bad_rows.len(), "총계는 실제 오류 수");

        let nums: Vec<usize> = r.errors.iter().map(|e| e.logical).collect();
        assert_eq!(nums, bad_rows[..limit], "행번호가 작은 쪽부터 남는다");
    }

    /// 같은 입력을 여러 번 스캔하면 항상 같은 결과가 나와야 한다. 상한이
    /// 스레드 도착 순서에 좌우되면 이 단언이 간헐적으로 깨진다.
    #[test]
    fn scan_lines_limit_is_deterministic() {
        const ROWS: usize = CHUNK * 4;
        let mut src = vec!["a,b".to_string()];
        for i in 0..ROWS {
            src.push(if i % 1_000 == 0 { "bad".into() } else { "ok,ok".into() });
        }
        let first = scan_lines(&src, b',', 2, 1, 37);
        for _ in 0..4 {
            assert_eq!(scan_lines(&src, b',', 2, 1, 37), first);
        }
        assert_eq!(first.errors.len(), 37);
        assert!(first.dropped > 0, "상한에 실제로 걸려야 의미 있는 검증");
    }

    #[test]
    fn scan_lines_empty_and_header_only() {
        assert_eq!(scan_lines(&[], b',', 3, 0, 100), ScanResult::default());
        // 헤더만 있는 파일 — 검사할 데이터 행이 없다.
        let src = lines(&["a,b,c"]);
        assert_eq!(scan_lines(&src, b',', 3, 1, 100), ScanResult::default());
    }

    #[test]
    fn counts_by_issue_type() {
        let src = lines(&[
            "a,b",
            "1",              // FieldCount
            "\"x,y",          // UnbalancedQuote
            "\u{FFFD},z",     // DecodeError
            "ok,ok",          // 정상
        ]);
        let r = scan_lines(&src, b',', 2, 1, 100);
        assert_eq!(r.errors.len(), 3);
        assert_eq!(r.counts(), (1, 1, 1));
    }

    // ---- scan_errors (mmap 경로) ----

    /// 파일을 만들어 인덱싱 완료까지 기다린 `(Source, LineIndex)`를 준다.
    fn open_indexed(content: &[u8]) -> (Arc<Source>, LineIndex) {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_val_{}_{}.csv", std::process::id(), id));
        std::fs::File::create(&p)
            .unwrap()
            .write_all(content)
            .unwrap();
        let src = Arc::new(crate::source::open(&p).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();
        crate::indexer::spawn_indexer(src.clone(), idx.clone(), Encoding::Utf8, ctx)
            .join()
            .unwrap();
        (src, idx)
    }

    #[test]
    fn scan_finds_bad_rows_with_logical_numbers() {
        let (src, idx) = open_indexed(b"a,b,c\n1,2,3\n4,5\n6,7,8\n");
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 3, 1, 100, None);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].logical, 2);
        assert_eq!(
            r.errors[0].issue,
            RowIssue::FieldCount { got: 2, expected: 3 }
        );
        assert_eq!(r.errors[0].preview, "4,5");
    }

    /// mmap 경로는 행 범위에 개행이 **포함**돼 온다. 떼지 않으면 미리보기 끝에
    /// `\r`이 남고 마지막 필드 값이 달라진다.
    #[test]
    fn scan_strips_crlf_from_preview() {
        let (src, idx) = open_indexed(b"a,b,c\r\n4,5\r\n");
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 3, 1, 100, None);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].preview, "4,5", "\\r이 남으면 안 된다");
    }

    #[test]
    fn scan_respects_data_start_skipping_header() {
        let (src, idx) = open_indexed(b"only_one_col\n1,2,3\n");
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 3, 1, 100, None);
        assert!(r.errors.is_empty(), "헤더 행은 검사에서 제외");
    }

    #[test]
    fn scan_results_sorted_by_logical() {
        let (src, idx) = open_indexed(b"a,b\n1\n2,3\n4\n5\n");
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 2, 1, 100, None);
        let nums: Vec<usize> = r.errors.iter().map(|e| e.logical).collect();
        assert_eq!(nums, vec![1, 3, 4]);
    }

    #[test]
    fn scan_limit_caps_collected_errors() {
        let (src, idx) = open_indexed(b"a,b\n1\n2\n3\n");
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 2, 1, 2, None);
        assert_eq!(r.errors.len(), 2);
        assert_eq!(r.dropped, 1);
    }

    /// 취소 신호가 서 있으면 아무것도 수집하지 않고 즉시 돌아온다.
    #[test]
    fn scan_cancel_stops_collection() {
        let (src, idx) = open_indexed(b"a,b\n1\n2\n3\n");
        let always: &(dyn Fn() -> bool + Sync) = &|| true;
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 2, 1, 100, Some(always));
        assert!(r.errors.is_empty(), "취소되면 수집하지 않는다");
    }

    /// 취소가 **청크 중간에서도** 걸려야 한다.
    ///
    /// 청크 경계에서만 확인하면 이미 시작한 청크는 `CHUNK`행(6.5만)을 전부
    /// 돈다. 여기서는 한 청크에 다 들어가는 크기의 입력을 주고, 스캔이 몇 행쯤
    /// 진행된 뒤 취소를 켜서 **끝까지 돌지 않는지** 본다.
    ///
    /// 관측은 "몇 행을 실제로 검사했는가"로 한다 — 판정식을 다시 계산하지 않고
    /// 프로덕션이 실행한 것을 센다.
    #[test]
    fn scan_cancel_stops_mid_chunk() {
        use std::sync::atomic::AtomicUsize;

        // 한 청크(CHUNK=65536)에 들어가면서 취소 확인 간격(1024)을 여러 번
        // 넘는 크기.
        const ROWS: usize = 20_000;
        let mut content = Vec::from(&b"a,b\n"[..]);
        for _ in 0..ROWS {
            content.extend_from_slice(b"bad\n"); // 전부 오류(필드 1개)
        }
        let (src, idx) = open_indexed(&content);

        // 취소 확인이 몇 번 불렸는지 세고, 일정 횟수 뒤 취소를 켠다.
        let calls = AtomicUsize::new(0);
        let trip: &(dyn Fn() -> bool + Sync) = &|| {
            // 첫 두 번(청크 진입 + 1024행)은 통과시키고 그 뒤로 취소.
            calls.fetch_add(1, Ordering::Relaxed) >= 2
        };
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 2, 1, usize::MAX, Some(trip));

        assert!(
            r.errors.len() < ROWS,
            "취소했는데 {}행을 전부 검사했다 — 청크 중간에서 안 멈춘다",
            r.errors.len()
        );
        assert!(
            calls.load(Ordering::Relaxed) > 1,
            "취소 확인이 청크당 한 번뿐이면 이 테스트는 무의미하다"
        );
    }

    /// 취소 신호가 꺼져 있으면 평소대로 전부 검사한다(위 테스트가 "항상 빈
    /// 결과"를 보는 것이 아님을 증명한다).
    #[test]
    fn scan_not_cancelled_collects_normally() {
        let (src, idx) = open_indexed(b"a,b\n1\n2\n3\n");
        let never: &(dyn Fn() -> bool + Sync) = &|| false;
        let r = scan_errors(&src, &idx, Encoding::Utf8, b',', 2, 1, 100, Some(never));
        assert_eq!(r.errors.len(), 3);
    }

    /// mmap 경로와 편집 버퍼 경로가 **같은 답**을 내야 한다. 한쪽만 고치면
    /// 편집 모드에 들어갔다 나올 때 오류 목록이 달라진다.
    #[test]
    fn mmap_and_lines_paths_agree() {
        let content = b"a,b,c\n1,2,3\n4,5\n\"x,y\n\n6,7,8,9\n";
        let (src, idx) = open_indexed(content);
        let from_mmap = scan_errors(&src, &idx, Encoding::Utf8, b',', 3, 1, 100, None);

        let text = String::from_utf8(content.to_vec()).unwrap();
        let mut buf: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        // 파일 끝 개행은 종결자다 — 뒤에 빈 줄을 만들지 않는다
        // (`edit::load_edit_buffer`와 같은 규칙).
        if text.ends_with('\n') {
            buf.pop();
        }
        let from_lines = scan_lines(&buf, b',', 3, 1, 100);

        assert_eq!(from_mmap, from_lines);
        assert!(!from_mmap.errors.is_empty(), "검증이 공집합을 비교하면 무의미");
    }

    // ---- ScanJob (백그라운드) ----

    #[test]
    fn spawn_scan_delivers_same_result_as_direct_scan() {
        let (src, idx) = open_indexed(b"a,b,c\n1,2,3\n4,5\n6,7,8\n");
        let direct = scan_errors(
            &src,
            &idx,
            Encoding::Utf8,
            b',',
            3,
            1,
            MAX_ROW_ERRORS,
            None,
        );
        let mut job = spawn_scan(
            src.clone(),
            idx.clone(),
            Encoding::Utf8,
            b',',
            3,
            1,
            egui::Context::default(),
        );
        // 완료까지 기다린다(테스트라 폴링해도 무방).
        let got = loop {
            if let Some(r) = job.take_result() {
                break r;
            }
            std::thread::yield_now();
        };
        assert_eq!(got, direct);
    }

    /// 완료 전에는 결과를 내주지 않는다(폴링 계약).
    #[test]
    fn take_result_is_none_until_finished() {
        let (src, idx) = open_indexed(b"a,b\n1\n");
        let mut job = spawn_scan(
            src,
            idx,
            Encoding::Utf8,
            b',',
            2,
            1,
            egui::Context::default(),
        );
        if !job.is_finished() {
            assert!(job.take_result().is_none());
        }
        // 완료되면 정확히 한 번만 내준다.
        while !job.is_finished() {
            std::thread::yield_now();
        }
        assert!(job.take_result().is_some());
        assert!(job.take_result().is_none(), "결과는 한 번만 꺼낸다");
    }

    /// 실파일 오류 검사 성능 + **전 행 차등 검증** 벤치.
    ///
    /// 병렬 스캔이 빠르기만 하고 틀리면 소용없으므로, 같은 파일을
    /// **단일 스레드 기준 구현**으로 한 번 더 훑어 결과를 대조한다. 기준
    /// 구현은 `check_line`을 그대로 쓰되 병렬·청크·병합을 거치지 않는다 —
    /// 검증 대상은 그 세 가지다.
    ///
    ///   TV_BENCH_FILE=... cargo test --release bench_validate -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_validate() {
        use std::time::Instant;
        let Ok(path) = std::env::var("TV_BENCH_FILE") else {
            eprintln!("TV_BENCH_FILE 미지정 — 스킵");
            return;
        };
        let path = std::path::PathBuf::from(path);
        let delim = match path.extension().and_then(|e| e.to_str()) {
            Some("tsv") | Some("tab") => b'\t',
            _ => b',',
        };

        let src = Arc::new(crate::source::open(&path).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();
        crate::indexer::spawn_indexer(src.clone(), idx.clone(), Encoding::Utf8, ctx)
            .join()
            .unwrap();
        let rows = idx.line_count();
        let gb = src.len() as f64 / 1e9;

        // 기대 컬럼 수 = 헤더 필드 수.
        let expected = {
            let (s, e) = idx.line_range(0).unwrap();
            let text = decode_line(src.slice(s, e), Encoding::Utf8);
            split_fields(text.trim_end_matches(['\r', '\n']), delim).len()
        };
        eprintln!(
            "file={} size={gb:.2}GB rows={rows} expected_cols={expected}",
            path.display()
        );

        // 상한을 사실상 없애 전수 비교가 되게 한다.
        let no_limit = usize::MAX;
        let t = Instant::now();
        let parallel = scan_errors(
            &src,
            &idx,
            Encoding::Utf8,
            delim,
            expected,
            1,
            no_limit,
            None,
        );
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "병렬 스캔: {ms:8.1} ms  ({:.1} M rows/s)  오류 {}행",
            rows as f64 / 1e6 / (ms / 1000.0),
            parallel.total()
        );

        // 단일 스레드 기준 구현.
        let (offsets, total_bytes) = idx.snapshot();
        let bytes = src.as_bytes();
        let t = Instant::now();
        let mut reference: Vec<RowError> = Vec::new();
        for logical in 1..rows {
            let Some((s, e)) = LineIndex::range_in(&offsets, total_bytes, logical) else {
                continue;
            };
            let text = decode_line(&bytes[s as usize..e as usize], Encoding::Utf8);
            let line = text.trim_end_matches(['\r', '\n']);
            if let Some(issue) = check_line(line, delim, expected) {
                reference.push(RowError { logical, issue, preview: preview_of(line) });
            }
        }
        let ref_ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "단일 스레드: {ref_ms:8.1} ms  (병렬이 {:.1}배 빠름)",
            ref_ms / ms
        );

        assert_eq!(
            parallel.errors, reference,
            "전 행 차등 검증 실패 — 병렬 결과가 기준과 다르다"
        );
        eprintln!("전 행 차등 검증: {} 행 중 불일치 0", rows - 1);
        let (fc, uq, de) = parallel.counts();
        eprintln!("유형별: 필드수 {fc} / 따옴표 {uq} / 디코드 {de}");

        // 상한 동작도 실파일에서 확인한다 — 상한이 걸린 목록은 (a) 오름차순이고
        // (b) 기준 결과의 **앞에서부터** 정확히 그만큼이어야 한다.
        if parallel.total() > 4 {
            let cap = parallel.total() / 2;
            let capped = scan_errors(
                &src,
                &idx,
                Encoding::Utf8,
                delim,
                expected,
                1,
                cap,
                None,
            );
            assert_eq!(capped.errors.len(), cap);
            assert_eq!(capped.total(), parallel.total(), "총계는 상한과 무관");
            assert_eq!(
                capped.errors,
                reference[..cap],
                "상한이 걸려도 앞에서부터 그대로"
            );
            eprintln!("상한 검증: {cap}개 수집 / {}개 버림 — 앞에서부터 일치", capped.dropped);
        }
    }

    /// 취소된 작업은 **부분 결과를 내주지 않는다**. 반쪽 목록을 "검사 완료"로
    /// 보여 주면 남은 오류를 없는 것으로 오해한다.
    #[test]
    fn cancelled_job_yields_no_result() {
        let (src, idx) = open_indexed(b"a,b\n1\n2\n3\n");
        let mut job = spawn_scan(
            src,
            idx,
            Encoding::Utf8,
            b',',
            2,
            1,
            egui::Context::default(),
        );
        job.cancel();
        while !job.is_finished() {
            std::thread::yield_now();
        }
        assert!(job.take_result().is_none());
    }
}
