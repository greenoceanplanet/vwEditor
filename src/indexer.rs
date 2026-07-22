use crate::index::{LineIndex, Phase};
use crate::parse::Encoding;
use crate::source::Source;
use std::sync::Arc;

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
///
/// 실제 인덱싱은 청크 경계를 처리하는 index_range가 담당한다. 이 함수는
/// 청크 없이 버퍼 전체를 한 번에 스캔하는 "정답지"로, 회귀 테스트가
/// index_range의 청크 경계 처리 결과를 이 함수와 대조하는 데 쓰인다.
#[allow(dead_code)]
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

const CHUNK: usize = 8 * 1024 * 1024; // 8MB

/// 한 워커가 담당하는 파일 구간 `[seg_start, seg_end)`의 스캔 결과.
pub struct SegmentResult {
    /// 이 구간이 담당하는 줄 시작 offset들(절대값), 오름차순.
    pub offsets: Vec<u64>,
    /// 이 구간을 끝까지 스캔했으면 true, 중단으로 중간에 멈췄으면 false.
    pub done: bool,
    /// "처리 완료"로 친 바이트 위치(절대값). done이면 seg_end.
    pub progress: u64,
}

/// 파일 구간 `[seg_start, seg_end)` 하나를 스캔해 그 구간이 담당하는 줄 시작
/// offset들을 로컬 Vec에 모아 반환한다. `LineIndex`에 직접 push하지 않으므로
/// 여러 워커가 동시에 호출해도 경합이 없다(순수 함수).
///
/// # 담당 규칙 (중복/누락 없이 구간을 concat하면 전체 순차 스캔과 동일)
/// - 개행의 "다음 위치"(= 줄 시작)가 `[seg_start, seg_end)` 안에 들어오는 개행만
///   담당한다. 즉 매치 시작 절대 위치가 `[seg_start, seg_end)` 인 개행을 처리해
///   `next_abs = 매치시작 + step` 을 push한다(단 `next_abs < total` 일 때만 —
///   파일 맨 끝 개행 뒤의 빈 줄은 세지 않음).
/// - 첫 줄 시작(0)은 `seg_start == 0` 인 구간(구간 0)만, 그리고 `total > 0` 일 때만
///   맨 앞에 push한다.
/// - 멀티바이트 개행(UTF-16, step=2)이 `seg_end` 직전에 걸칠 수 있으므로 스캔
///   슬라이스는 `seg_end` 를 `step-1` 만큼 내다본(peek) `scan_end`(단 total 초과
///   금지)까지 잡는다. 담당 판정은 여전히 매치 시작이 `< seg_end` 인지로 한다.
///
/// # 중단
/// `chunk_size` 단위로 `should_pause()` 를 확인해 서 있으면 즉시 멈추고
/// `done = false`, `progress = 그때까지 처리한 위치` 로 반환한다.
pub fn scan_segment(
    bytes: &[u8],
    seg_start: usize,
    seg_end: usize,
    enc: Encoding,
    chunk_size: usize,
    mut should_pause: impl FnMut() -> bool,
    mut on_chunk: impl FnMut(usize),
) -> SegmentResult {
    let total = bytes.len();
    let pat = newline_pattern(enc);
    let step = pat.len();
    let mut offsets = Vec::new();

    // 첫 줄 시작(0)은 구간 0만, 파일이 비어있지 않을 때만.
    if seg_start == 0 && total > 0 {
        offsets.push(0);
    }

    // 개행 패턴 안에서 0x0A(LF) 바이트의 위치. memchr로 0x0A를 찾은 뒤
    // 이 오프셋만큼 뒤로 물러난 곳이 패턴 시작 후보다.
    //   UTF-8/CP949: [0x0A]        → lf_off = 0
    //   UTF-16LE   : [0x0A, 0x00]  → lf_off = 0
    //   UTF-16BE   : [0x00, 0x0A]  → lf_off = 1
    let lf_off = pat.iter().position(|&b| b == 0x0A).unwrap();

    let mut pos = seg_start;
    while pos < seg_end {
        if should_pause() {
            return SegmentResult {
                offsets,
                done: false,
                progress: pos as u64,
            };
        }
        let end = (pos + chunk_size).min(seg_end);
        // 청크 경계에서 멀티바이트 개행이 잘리지 않도록 step-1 만큼 내다본다.
        let scan_end = (end + step.saturating_sub(1)).min(total);
        let slice = &bytes[pos..scan_end];
        let limit = end - pos; // 매치 "시작" 위치는 이 청크 몫([pos,end))이어야 함

        // memchr로 0x0A(LF) 후보를 SIMD로 빠르게 찾는다. 각 후보 m에 대해
        // 패턴 시작 = m - lf_off 을 계산하고, 그 시작이 이 청크 몫([0,limit))
        // 안이며 slice에서 패턴 전체와 일치할 때만 개행으로 인정한다.
        for m in memchr::memchr_iter(0x0A, slice) {
            // 패턴 시작이 음수가 되는 경우(예: BE에서 0x0A가 slice 맨 앞) 스킵.
            let start = match m.checked_sub(lf_off) {
                Some(s) => s,
                None => continue,
            };
            if start >= limit {
                continue; // 매치 "시작"이 이 청크 몫 밖(다음 청크가 담당)
            }
            if start + step > slice.len() {
                continue; // 패턴이 slice(peek 포함) 끝을 넘어감 → 불완전
            }
            if &slice[start..start + step] != pat {
                continue; // 0x0A는 있으나 전체 패턴은 아님(예: LE인데 다음이 0x00 아님)
            }
            let next_abs = pos + start + step;
            if next_abs < total {
                offsets.push(next_abs as u64);
            }
        }

        // 이 청크에서 방금 처리한 바이트 수를 진행 콜백에 알린다(진행률 합산용).
        on_chunk(end - pos);
        pos = end;
    }

    SegmentResult {
        offsets,
        done: true,
        progress: seg_end as u64,
    }
}

/// 여러 구간 스캔 결과를 병합한다. 정상 완주와 중단을 하나의 규칙으로 처리:
/// 구간 0부터 순서대로 훑되, **완료된 구간은 통째로** 이어붙이고 **첫 번째
/// 미완료 구간을 만나면 그 구간이 그때까지 스캔한 부분까지만** 넣고 멈춘다
/// (그 뒤 구간들은 앞이 비어 연속성이 깨지므로 버림).
///
/// - 모든 구간 완료 → 전체 concat, `all_done = true`, `bytes_done = total`.
/// - 중단 → 맨 앞부터 연속 완료된 prefix까지만, `all_done = false`,
///   `bytes_done = 첫 미완료 구간의 progress`(= 연속으로 스캔된 바이트 경계).
///
/// 구간은 파일 순서(seg_start 오름차순)대로 `segments`에 들어있다고 가정한다.
/// 반환: `(offsets, all_done, bytes_done)`.
fn merge_segments(segments: &[SegmentResult]) -> (Vec<u64>, bool, u64) {
    let mut offsets = Vec::new();
    for seg in segments {
        offsets.extend_from_slice(&seg.offsets);
        if !seg.done {
            // 첫 미완료 구간: 그때까지만 담고 여기서 멈춘다(뒤는 버림).
            return (offsets, false, seg.progress);
        }
    }
    // 전부 완료. bytes_done은 마지막 구간의 progress(= total).
    let bytes_done = segments.last().map(|s| s.progress).unwrap_or(0);
    (offsets, true, bytes_done)
}

/// 프라이밍 구간 크기: 맨 앞 이만큼을 즉시 순차 스캔해 첫 화면을 띄운다.
/// (window×5행보다 넉넉히 잡아 어떤 창 높이에서도 첫 화면이 채워지게 함.)
const PRIME_BYTES: usize = 2 * 1024 * 1024; // 2MB

/// 파일 맨 앞 `PRIME_BYTES` 구간을 즉시 스캔해 인덱스를 채운다(첫 화면용).
/// 병렬 스캔 전에 호출되어, 맨 앞 offset이 먼저 인덱스에 들어가 첫 화면이
/// 즉시 표시되게 한다. 이후 병렬 스캔이 완료되면 전체 결과로 교체된다.
fn prime(bytes: &[u8], index: &LineIndex, enc: Encoding) {
    let total = bytes.len();
    let prime_end = PRIME_BYTES.min(total);
    let res = scan_segment(bytes, 0, prime_end, enc, CHUNK, || false, |_| {});
    index.replace_offsets(res.offsets);
    index.set_bytes_done(prime_end as u64);
}

/// 파일 전체 `[0, total)`을 워커 수만큼 구간으로 나눠 rayon으로 동시에 스캔한다.
/// 각 워커는 자기 구간을 `scan_segment`로 스캔하고, 그 결과를 `merge_segments`가
/// 순서대로 병합한다. 진행 바이트는 모든 워커가 `AtomicU64`에 합산해 상태바에
/// 실시간 반영한다. 반환된 `(offsets, all_done, bytes_done)`을 호출측이 인덱스에
/// 반영한다.
fn parallel_scan(
    bytes: &[u8],
    enc: Encoding,
    index: &LineIndex,
    ctx: &egui::Context,
) -> (Vec<u64>, bool, u64) {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    let total = bytes.len();
    if total == 0 {
        return (Vec::new(), true, 0);
    }

    // 워커 수 = 가용 병렬성(코어 수). 파일이 작으면 구간이 total보다 많지 않게 클램프.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1)
        .min(total); // 최소 1바이트/구간

    // 진행 합산용 원자 카운터. 각 워커가 청크마다 처리 바이트를 더한다.
    let progress = AtomicU64::new(0);

    // 구간 경계: [k*total/workers, (k+1)*total/workers)
    let segments: Vec<SegmentResult> = (0..workers)
        .into_par_iter()
        .map(|k| {
            let seg_start = k * total / workers;
            let seg_end = (k + 1) * total / workers;
            scan_segment(
                bytes,
                seg_start,
                seg_end,
                enc,
                CHUNK,
                || index.pause_requested(),
                |chunk_bytes| {
                    let done = progress.fetch_add(chunk_bytes as u64, Ordering::Relaxed)
                        + chunk_bytes as u64;
                    index.set_bytes_done(done);
                    ctx.request_repaint();
                },
            )
        })
        .collect();

    merge_segments(&segments)
}

/// 백그라운드 스레드에서 (1) 프라이밍으로 첫 화면을 즉시 띄우고,
/// (2) 파일 전체를 rayon으로 병렬 스캔해 인덱싱한다.
/// 중단(pause_requested)이 서면 맨 앞부터 연속 완료된 prefix까지만 반영하고
/// `Paused`로 멈춘다. 재개는 호출측(app)이 인덱스를 클리어하고 새로 spawn한다.
pub fn spawn_indexer(
    source: Arc<Source>,
    index: LineIndex,
    enc: Encoding,
    ctx: egui::Context,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let bytes = source.as_bytes();
        let total = bytes.len();

        // (1) 프라이밍: 맨 앞을 즉시 스캔해 첫 화면 표시.
        index.set_phase(Phase::Priming);
        prime(bytes, &index, enc);
        ctx.request_repaint();

        // 프라이밍이 파일 전체를 이미 덮었으면(작은 파일) 바로 완료.
        if total <= PRIME_BYTES {
            index.set_bytes_done(total as u64);
            index.set_phase(Phase::Complete);
            ctx.request_repaint();
            return;
        }

        // 프라이밍 직후 이미 중단 요청이 있었다면 병렬 진입 없이 멈춘다.
        if index.pause_requested() {
            index.clear_pause();
            index.set_phase(Phase::Paused);
            ctx.request_repaint();
            return;
        }

        // (2) 병렬 스캔.
        index.set_phase(Phase::Indexing);
        let (offsets, all_done, bytes_done) = parallel_scan(bytes, enc, &index, &ctx);
        index.replace_offsets(offsets);
        index.set_bytes_done(bytes_done);

        if all_done {
            index.set_phase(Phase::Complete);
        } else {
            index.clear_pause();
            index.set_phase(Phase::Paused);
        }
        ctx.request_repaint();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{LineIndex, Phase};
    use crate::source;
    use std::io::Write;
    use std::sync::Arc;

    fn temp_file(content: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_idx_{}_{}.txt", std::process::id(), id));
        std::fs::File::create(&p).unwrap().write_all(content).unwrap();
        p
    }

    #[test]
    fn indexer_indexes_all_lines() {
        let p = temp_file(b"a\nb\nc\nd\n");
        let src = Arc::new(source::open(&p).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();
        let handle = spawn_indexer(src, idx.clone(), Encoding::Utf8, ctx);
        handle.join().unwrap();
        assert_eq!(idx.status().phase, Phase::Complete);
        // "a\nb\nc\nd\n" → 줄 시작: 0,2,4,6 (마지막 개행 뒤 빈 줄은 포함 안 함)
        assert_eq!(idx.line_count(), 4);
    }

    /// 여러 구간으로 나눠 scan_segment한 결과를 순서대로 concat하면
    /// 순차 scan_offsets(정답지)와 정확히 일치해야 한다 (담당 규칙의 핵심 불변식).
    /// 다양한 구간 수/경계 위치에서 확인한다.
    fn assert_segments_match_oracle(bytes: &[u8], enc: Encoding, seg_count: usize) {
        let expected = scan_offsets(bytes, 0, enc);
        let total = bytes.len();
        let mut got = Vec::new();
        for k in 0..seg_count {
            let seg_start = k * total / seg_count;
            let seg_end = (k + 1) * total / seg_count;
            // 작은 chunk_size로도 동일해야 함(멀티바이트 개행 경계 스트래들 강제).
            let res = scan_segment(bytes, seg_start, seg_end, enc, 3, || false, |_| {});
            assert!(res.done);
            assert_eq!(res.progress, seg_end as u64);
            got.extend(res.offsets);
        }
        assert_eq!(
            got, expected,
            "seg_count={seg_count} 에서 concat 결과가 순차 스캔과 다름"
        );
    }

    #[test]
    fn segments_concat_matches_oracle_utf8() {
        let bytes = b"ab\ncd\nef\ngh\nij\nkl\n";
        for seg_count in [1, 2, 3, 4, 5, 7] {
            assert_segments_match_oracle(bytes, Encoding::Utf8, seg_count);
        }
    }

    #[test]
    fn segments_concat_matches_oracle_utf8_no_trailing_newline() {
        // 마지막에 개행이 없는 경우(마지막 줄 시작이 마지막 개행 다음).
        let bytes = b"ab\ncd\nef\ngh";
        for seg_count in [1, 2, 3, 4] {
            assert_segments_match_oracle(bytes, Encoding::Utf8, seg_count);
        }
    }

    #[test]
    fn segments_concat_matches_oracle_utf16le() {
        // "ab\ncd\nef\ngh\n" UTF-16LE (한 줄 6바이트). 멀티바이트 개행 경계 검증.
        let mut bytes = Vec::new();
        for pair in ["ab", "cd", "ef", "gh"] {
            for c in pair.chars() {
                let code = c as u16;
                bytes.push((code & 0xFF) as u8);
                bytes.push((code >> 8) as u8);
            }
            bytes.extend_from_slice(&[0x0A, 0x00]);
        }
        for seg_count in [1, 2, 3, 4, 6, 8] {
            assert_segments_match_oracle(&bytes, Encoding::Utf16Le, seg_count);
        }
    }

    #[test]
    fn merge_all_done_concats_everything() {
        let segs = vec![
            SegmentResult { offsets: vec![0, 3], done: true, progress: 6 },
            SegmentResult { offsets: vec![6, 9], done: true, progress: 12 },
            SegmentResult { offsets: vec![12], done: true, progress: 15 },
        ];
        let (offs, all_done, bytes_done) = merge_segments(&segs);
        assert_eq!(offs, vec![0, 3, 6, 9, 12]);
        assert!(all_done);
        assert_eq!(bytes_done, 15);
    }

    #[test]
    fn merge_stops_at_first_incomplete_segment() {
        // 구간 0 완료, 구간 1 미완료(중단), 구간 2는 완료됐어도 버려야 함.
        // → 맨 앞부터 연속된 prefix(구간0 + 구간1의 부분)까지만, 구멍 없음.
        let segs = vec![
            SegmentResult { offsets: vec![0, 3], done: true, progress: 6 },
            SegmentResult { offsets: vec![6], done: false, progress: 8 }, // 중단됨
            SegmentResult { offsets: vec![12, 15], done: true, progress: 18 }, // 버려짐
        ];
        let (offs, all_done, bytes_done) = merge_segments(&segs);
        assert_eq!(offs, vec![0, 3, 6], "구간2(12,15)는 앞이 비어 버려져야 함");
        assert!(!all_done);
        assert_eq!(bytes_done, 8, "연속으로 스캔된 바이트 경계 = 첫 미완료 구간 progress");
    }

    #[test]
    fn merge_first_segment_incomplete() {
        // 맨 앞 구간부터 미완료면 그 부분까지만.
        let segs = vec![
            SegmentResult { offsets: vec![0], done: false, progress: 2 },
            SegmentResult { offsets: vec![6, 9], done: true, progress: 12 },
        ];
        let (offs, all_done, bytes_done) = merge_segments(&segs);
        assert_eq!(offs, vec![0]);
        assert!(!all_done);
        assert_eq!(bytes_done, 2);
    }

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

    /// 회귀 테스트: UTF-16LE 개행 패턴(2바이트, [0x0A,0x00])이 청크 경계에
    /// 정확히 걸치도록(첫 바이트가 청크의 마지막 바이트) 구성한 버퍼를
    /// 아주 작은 chunk_size로 `scan_segment`에 넣고 스캔한다.
    ///
    /// step-1 peek(청크 경계에서 step-1 바이트를 내다보는 로직)이 제거되면
    /// 이 스트래들 개행이 누락되어 아래 assert가 실패해야 한다. 실제로
    /// 이 테스트를 작성하며 peek 로직을 임시로 제거해 실패하는 것을
    /// 확인했다(task-9-report.md 참고).
    #[test]
    fn indexer_utf16le_newline_straddles_chunk_boundary() {
        // 각 "줄"은 UTF-16LE 로 2글자 + 개행(0x0A,0x00) = 6바이트.
        // "ab\ncd\nef\ngh\n" 를 UTF-16LE로 인코딩한 버퍼:
        //   a b \n c d \n e f \n g h \n  (각 글자 2바이트, \n = 0A 00)
        // 바이트 레이아웃(총 24바이트, 0-indexed):
        //  0: 61 00  (a)
        //  2: 62 00  (b)
        //  4: 0A 00  (\n)   <- 개행의 첫 바이트가 인덱스 4.
        //  6: 63 00  (c)
        //  8: 64 00  (d)
        // 10: 0A 00  (\n)
        // 12: 65 00  (e)
        // 14: 66 00  (f)
        // 16: 0A 00  (\n)
        // 18: 67 00  (g)
        // 20: 68 00  (h)
        // 22: 0A 00  (\n)
        //
        // chunk_size=5 로 스캔하면 첫 청크는 [0,5). 개행의 첫 바이트(인덱스4)는
        // `< end`(5) 이므로 매치 "시작" 조건은 만족하지만, 두 번째 바이트
        // (인덱스5)는 청크 밖에 있다. peek 로직(step-1=1 만큼 내다봄)이 있어야
        // scan_end=6 이 되어 slice가 인덱스5까지 포함해 패턴이 온전히 매치된다.
        // peek 이 없으면(scan_end=end=5) slice 길이가 5뿐이라
        // `i+step<=slice.len()`(1+2<=5? 4<=5 는 참이지만 slice[4..6]은 범위 밖이라
        // 실제로는 slice.len()==5이므로 i=4일 때 4+2=6 <= 5 가 거짓이 되어 매치
        // 자체가 스킵되고, 그 결과 개행이 통째로 누락된다.
        let mut bytes = Vec::new();
        for ch_pair in ["ab", "cd", "ef", "gh"] {
            for c in ch_pair.chars() {
                let mut buf = [0u8; 2];
                let code = c as u16;
                buf[0] = (code & 0xFF) as u8;
                buf[1] = (code >> 8) as u8;
                bytes.extend_from_slice(&buf);
            }
            bytes.extend_from_slice(&[0x0A, 0x00]); // UTF-16LE '\n'
        }
        assert_eq!(bytes.len(), 24);

        // 정답(un-chunked 스캔): 전체 버퍼를 한 번에 scan_offsets로 스캔.
        let expected = scan_offsets(&bytes, 0, Encoding::Utf16Le);
        assert_eq!(expected, vec![0, 6, 12, 18]);

        // chunk_size=5로 한 구간 전체를 scan_segment(peek 로직 포함)로 스캔.
        let res = scan_segment(
            &bytes,
            0,
            bytes.len(),
            Encoding::Utf16Le,
            5, // 작은 chunk_size로 경계 스트래들을 강제 재현
            || false, // pause 없음
            |_| {},
        );
        assert!(res.done);
        assert_eq!(res.offsets, expected);
        assert_eq!(res.offsets.len(), 4);
    }

    /// 병렬 인덱싱 도중 pause를 걸면 Paused로 멈추고, 남은 offset들은 맨 앞부터
    /// 연속된 prefix여야 한다(구멍 없음). pause를 처음부터 세워두면 프라이밍
    /// 직후 병렬 진입 전에 멈추므로, 병렬 진입 후 멈추도록 프라이밍 완료를
    /// 기다렸다가 pause를 건다.
    #[test]
    fn indexer_pause_yields_contiguous_prefix() {
        // PRIME_BYTES(2MB)보다 훨씬 큰 파일로 병렬 경로를 타게 한다.
        let unit: &[u8] = b"abcd\n"; // 5바이트/줄
        let reps = (PRIME_BYTES * 4) / unit.len(); // 약 8MB
        let mut bytes = Vec::with_capacity(reps * unit.len());
        for _ in 0..reps {
            bytes.extend_from_slice(unit);
        }
        let expected_full = scan_offsets(&bytes, 0, Encoding::Utf8);

        let mut p = std::env::temp_dir();
        p.push(format!(
            "tv_idx_pause_{}_{}.txt",
            std::process::id(),
            bytes.len()
        ));
        std::fs::File::create(&p).unwrap().write_all(&bytes).unwrap();
        let src = Arc::new(source::open(&p).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();

        // 프라이밍이 끝나 병렬 단계에 들어갈 때까지 잠깐 기다렸다가 pause 요청.
        let idx2 = idx.clone();
        let handle = spawn_indexer(src, idx.clone(), Encoding::Utf8, ctx);
        // Priming을 지나 Indexing이 되면 pause 요청(타이밍에 관계없이 정확성은
        // 보장됨 — 병렬 진입 전에 걸리면 Paused로 프라이밍분만, 후에 걸리면
        // 연속 prefix까지. 어느 경우든 "연속 prefix" 불변식은 성립).
        while idx2.status().phase == Phase::Priming {
            std::hint::spin_loop();
        }
        idx2.request_pause();
        handle.join().unwrap();
        let _ = std::fs::remove_file(&p);

        let phase = idx.status().phase;
        assert!(
            phase == Phase::Paused || phase == Phase::Complete,
            "pause 후에는 Paused(또는 경쟁적으로 이미 Complete)여야 함, got {phase:?}"
        );

        // 남은 offset들이 전체 정답지의 "맨 앞부터 연속된 prefix"인지 확인.
        let got: Vec<u64> = (0..idx.line_count())
            .map(|i| idx.offset(i).unwrap())
            .collect();
        assert_eq!(
            got,
            expected_full[..got.len()].to_vec(),
            "남은 offset은 정답지의 연속 prefix여야 함(구멍 없음)"
        );
    }

    /// spawn_indexer 자체가 멀티 청크(실 8MB CHUNK 상수)를 지나는 큰 파일과
    /// UTF-16LE 인코딩에 대해서도 scan_offsets(un-chunked ground truth)와
    /// 일치하는 결과를 내는지 검증한다. 파일 크기를 CHUNK의 배수 부근으로
    /// 잡아 실제 청크 경계를 여러 번 통과하게 한다.
    #[test]
    fn indexer_multi_chunk_utf16le_matches_scan_offsets() {
        // "ab\n" 반복을 UTF-16LE로 인코딩하면 한 반복이 6바이트.
        // CHUNK(8MB)의 배수 부근을 넘도록 총 3청크 분량(약 24MB)을 만든다.
        // 6으로 나누어떨어지는 반복 수를 사용해 개행 경계가 정확히 맞도록 함.
        let unit: &[u8] = &[0x61, 0x00, 0x62, 0x00, 0x0A, 0x00]; // "ab\n" UTF-16LE
        let target_len = (CHUNK * 3) + 1234 * 6; // 청크 3개를 넘고 딱 떨어지지 않는 길이
        let reps = target_len / unit.len();
        let mut bytes = Vec::with_capacity(reps * unit.len());
        for _ in 0..reps {
            bytes.extend_from_slice(unit);
        }

        let expected = scan_offsets(&bytes, 0, Encoding::Utf16Le);

        let mut p = std::env::temp_dir();
        p.push(format!("tv_idx_multichunk_{}.txt", bytes.len()));
        std::fs::File::create(&p).unwrap().write_all(&bytes).unwrap();
        let src = Arc::new(source::open(&p).unwrap());
        let idx = LineIndex::new(src.len());
        let ctx = egui::Context::default();
        let handle = spawn_indexer(src, idx.clone(), Encoding::Utf16Le, ctx);
        handle.join().unwrap();
        let _ = std::fs::remove_file(&p);

        assert_eq!(idx.status().phase, Phase::Complete);
        let got: Vec<u64> = (0..idx.line_count())
            .map(|i| idx.offset(i).unwrap())
            .collect();
        assert_eq!(got, expected);
        assert_eq!(idx.line_count(), reps);
    }
}
