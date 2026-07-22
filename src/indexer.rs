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

/// `spawn_indexer`의 핵심 청크 스캔 루프. `chunk_size`를 파라미터로 받아
/// 테스트에서 작은 값(예: 4바이트)을 넣어 청크 경계 스트래들 버그를
/// 재현/회귀 검증할 수 있게 한다. 실 운용 경로(`spawn_indexer`)는 항상
/// `CHUNK`(8MB)를 넘긴다.
///
/// 청크 경계 안전성: 청크를 `[pos, end)` 로만 스캔하면, 멀티바이트 개행 패턴
/// (UTF-16LE/BE, step=2)이 `end` 바로 앞뒤로 걸쳐 있을 때 그 패턴이 통째로
/// 스캔 창 밖으로 밀려 누락될 수 있다(패턴의 첫 바이트가 이 청크의 마지막
/// 바이트인 경우). 이를 막기 위해 스캔에 사용하는 슬라이스는 `end` 를
/// `step - 1` 바이트만큼 앞으로 내다본(peek) `scan_end`(단, total을 넘지
/// 않음)까지 잡아 걸친 패턴도 온전히 매치되게 하고, 실제로 "처리 완료"로
/// 치는 `pos` 전진은 여전히 `end` 까지만 한다. 다음 청크가 같은 패턴을
/// 다시 스캔하더라도 push_offset은 오직 매치 시작 위치가 `[pos, end)` 안에
/// 있을 때만 하므로 중복 offset은 발생하지 않는다.
fn index_range(
    bytes: &[u8],
    index: &LineIndex,
    enc: Encoding,
    chunk_size: usize,
    mut pos: usize,
    mut should_pause: impl FnMut() -> bool,
    mut on_chunk_done: impl FnMut(usize),
) -> bool {
    let total = bytes.len();
    let pat = newline_pattern(enc);
    let step = pat.len();

    // 첫 줄 시작(0)은 인덱스가 비어있을 때만 추가.
    if index.line_count() == 0 && total > 0 {
        index.push_offset(0);
    }

    while pos < total {
        if should_pause() {
            return true; // paused
        }
        let end = (pos + chunk_size).min(total);
        // 청크 경계에서 개행 패턴이 잘리지 않도록 step-1 만큼 내다보고 스캔한다.
        // (patterns whose match start is < end are fully found even if they
        // straddle `end`; pos still advances only to `end`.)
        let scan_end = (end + step.saturating_sub(1)).min(total);
        let slice = &bytes[pos..scan_end];
        let mut i = 0;
        let limit = end - pos; // 매치 "시작" 위치는 이 청크 몫([pos,end))이어야 함
        while i < limit {
            if i + step <= slice.len() && &slice[i..i + step] == pat {
                let next_abs = pos + i + step;
                if next_abs < total {
                    index.push_offset(next_abs as u64);
                }
                i += step;
            } else {
                i += 1;
            }
        }
        pos = end;
        on_chunk_done(pos);
    }
    false // completed, not paused
}

/// 백그라운드 스레드에서 파일 끝까지 개행 offset을 점진적으로 인덱싱한다.
/// pause_requested가 서면 Paused로 멈추고 스레드 종료(재개는 새 spawn).
pub fn spawn_indexer(
    source: Arc<Source>,
    index: LineIndex,
    enc: Encoding,
    ctx: egui::Context,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        index.set_phase(Phase::Indexing);
        let bytes = source.as_bytes();
        let total = bytes.len();
        // 이미 인덱싱된 만큼 이어서 시작(재개 지원): "다음 스캔 시작 바이트"를
        // bytes_done 기준으로 삼는다.
        let pos = (index.status().bytes_done as usize).min(total);

        let paused = index_range(
            bytes,
            &index,
            enc,
            CHUNK,
            pos,
            || index.pause_requested(),
            |done_pos| {
                index.set_bytes_done(done_pos as u64);
                ctx.request_repaint();
            },
        );

        if paused {
            index.clear_pause();
            index.set_phase(Phase::Paused);
            return;
        }
        index.set_bytes_done(total as u64);
        index.set_phase(Phase::Complete);
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
        let mut p = std::env::temp_dir();
        p.push(format!("tv_idx_{}.txt", content.len()));
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
    /// 아주 작은 chunk_size로 `index_range`에 넣고 스캔한다.
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

        // chunk_size=5로 청크 분할 스캔(peek 로직이 있으면 expected와 동일해야 함).
        let idx = LineIndex::new(bytes.len() as u64);
        let paused = index_range(
            &bytes,
            &idx,
            Encoding::Utf16Le,
            5, // 작은 chunk_size로 경계 스트래들을 강제 재현
            0,
            || false, // pause 없음
            |_done_pos| {},
        );
        assert!(!paused);

        let got: Vec<u64> = (0..idx.line_count())
            .map(|i| idx.offset(i).unwrap())
            .collect();
        assert_eq!(got, expected);
        assert_eq!(idx.line_count(), 4);
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
