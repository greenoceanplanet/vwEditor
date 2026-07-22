use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Priming,
    Indexing,
    Paused,
    Complete,
}

#[derive(Debug, Clone, Copy)]
pub struct IndexStatus {
    pub phase: Phase,
    pub bytes_done: u64,
    pub total_bytes: u64,
}

struct Inner {
    offsets: RwLock<Vec<u64>>,
    total_bytes: u64,
    bytes_done: AtomicU64,
    phase: RwLock<Phase>,
    pause: AtomicBool,
}

/// 줄 시작 offset 배열 + 인덱싱 상태. Arc 공유(Clone은 Arc clone).
#[derive(Clone)]
pub struct LineIndex {
    inner: Arc<Inner>,
}

impl LineIndex {
    pub fn new(total_bytes: u64) -> LineIndex {
        LineIndex {
            inner: Arc::new(Inner {
                offsets: RwLock::new(Vec::new()),
                total_bytes,
                bytes_done: AtomicU64::new(0),
                phase: RwLock::new(Phase::Priming),
                pause: AtomicBool::new(false),
            }),
        }
    }

    /// offset 하나를 뒤에 추가. 병렬 인덱싱 전환 후 프로덕션 경로는
    /// `replace_offsets`(벌크 교체)를 쓰므로, 이 단건 API는 현재 테스트에서
    /// offset 배열을 구성하는 데만 쓰인다. API 대칭성을 위해 남겨둔다.
    #[allow(dead_code)]
    pub fn push_offset(&self, off: u64) {
        self.inner.offsets.write().unwrap().push(off);
    }

    /// offset 배열 전체를 한 번의 락으로 교체한다. 병렬 스캔 결과(전체 파일의
    /// 완전한 offset 배열)를 반영할 때 사용한다. 프라이밍이 채운 앞부분은
    /// 병렬 결과의 접두부와 동일하므로, 교체해도 이음새 없이 이어진다.
    /// 프라이밍 도중(교체 전)에는 기존 앞부분이 남아 첫 화면이 유지된다.
    pub fn replace_offsets(&self, offsets: Vec<u64>) {
        *self.inner.offsets.write().unwrap() = offsets;
    }

    pub fn line_count(&self) -> usize {
        self.inner.offsets.read().unwrap().len()
    }

    /// 현재 UI는 line_range만 사용하지만, offset 조회 API를 대칭적으로 남겨둔다.
    #[allow(dead_code)]
    pub fn offset(&self, row: usize) -> Option<u64> {
        self.inner.offsets.read().unwrap().get(row).copied()
    }

    /// 해당 행의 byte 범위 [start, end). 마지막 행이면 end = total_bytes.
    pub fn line_range(&self, row: usize) -> Option<(u64, u64)> {
        let offsets = self.inner.offsets.read().unwrap();
        let start = *offsets.get(row)?;
        let end = offsets
            .get(row + 1)
            .copied()
            .unwrap_or(self.inner.total_bytes);
        Some((start, end))
    }

    pub fn set_phase(&self, phase: Phase) {
        *self.inner.phase.write().unwrap() = phase;
    }

    pub fn set_bytes_done(&self, n: u64) {
        self.inner.bytes_done.store(n, Ordering::Relaxed);
    }

    pub fn status(&self) -> IndexStatus {
        IndexStatus {
            phase: *self.inner.phase.read().unwrap(),
            bytes_done: self.inner.bytes_done.load(Ordering::Relaxed),
            total_bytes: self.inner.total_bytes,
        }
    }

    pub fn request_pause(&self) {
        self.inner.pause.store(true, Ordering::Relaxed);
    }

    pub fn pause_requested(&self) -> bool {
        self.inner.pause.load(Ordering::Relaxed)
    }

    pub fn clear_pause(&self) {
        self.inner.pause.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_count() {
        let idx = LineIndex::new(100);
        idx.push_offset(0);
        idx.push_offset(10);
        idx.push_offset(20);
        assert_eq!(idx.line_count(), 3);
    }

    #[test]
    fn line_range_uses_next_offset() {
        let idx = LineIndex::new(100);
        idx.push_offset(0);
        idx.push_offset(10);
        idx.push_offset(25);
        // 0행: [0,10), 1행: [10,25)
        assert_eq!(idx.line_range(0), Some((0, 10)));
        assert_eq!(idx.line_range(1), Some((10, 25)));
    }

    #[test]
    fn last_line_range_uses_total_bytes() {
        let idx = LineIndex::new(30);
        idx.push_offset(0);
        idx.push_offset(10);
        // 마지막 행(1)은 다음 offset이 없으니 total_bytes(30)까지
        assert_eq!(idx.line_range(1), Some((10, 30)));
    }

    #[test]
    fn replace_offsets_swaps_whole_array() {
        let idx = LineIndex::new(100);
        idx.push_offset(0);
        idx.push_offset(10);
        assert_eq!(idx.line_count(), 2);
        // 프라이밍이 채운 앞부분을 병렬 완료 결과로 통째 교체.
        idx.replace_offsets(vec![0, 10, 20, 30, 40]);
        assert_eq!(idx.line_count(), 5);
        assert_eq!(idx.line_range(4), Some((40, 100)));
    }

    #[test]
    fn pause_flag_roundtrip() {
        let idx = LineIndex::new(0);
        assert!(!idx.pause_requested());
        idx.request_pause();
        assert!(idx.pause_requested());
        idx.clear_pause();
        assert!(!idx.pause_requested());
    }

    #[test]
    fn status_reflects_setters() {
        let idx = LineIndex::new(1000);
        idx.set_phase(Phase::Indexing);
        idx.set_bytes_done(400);
        let st = idx.status();
        assert_eq!(st.phase, Phase::Indexing);
        assert_eq!(st.bytes_done, 400);
        assert_eq!(st.total_bytes, 1000);
    }
}
