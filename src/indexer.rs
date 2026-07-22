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

/// 백그라운드 스레드에서 파일 끝까지 개행 offset을 점진적으로 인덱싱한다.
/// pause_requested가 서면 Paused로 멈추고 스레드 종료(재개는 새 spawn).
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
        let mut pos = (index.status().bytes_done as usize).min(total);
        let pat = newline_pattern(enc);
        let step = pat.len();

        // 첫 줄 시작(0)은 인덱스가 비어있을 때만 추가.
        if index.line_count() == 0 && total > 0 {
            index.push_offset(0);
        }

        while pos < total {
            if index.pause_requested() {
                index.clear_pause();
                index.set_phase(Phase::Paused);
                return;
            }
            let end = (pos + CHUNK).min(total);
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
            index.set_bytes_done(pos as u64);
            ctx.request_repaint();
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
}
