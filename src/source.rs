use memmap2::Mmap;
use std::path::Path;

/// mmap으로 매핑된 읽기 전용 파일. 파일 크기와 무관하게 즉시 매핑된다.
///
/// Windows에서는 길이 0인 실제 파일을 `Mmap::map`으로도, `MmapOptions::map_anon`으로도
/// 매핑할 수 없다(둘 다 `ERROR_INVALID_PARAMETER`로 실패하는 것을 확인). 따라서 빈 파일은
/// `mmap: None`으로 표현하고, `len`/`slice`/`as_bytes`가 이를 빈 슬라이스로 처리한다.
pub struct Source {
    mmap: Option<Mmap>,
    /// 파일 없이 메모리 바이트로 만든 소스. mmap과 배타적.
    /// (찾기 결과 행 추출처럼 디스크에 대응하는 파일이 없는 문서용.)
    owned: Option<Vec<u8>>,
}

/// 파일을 열어 메모리 매핑한다. 빈 파일도 허용.
pub fn open(path: &Path) -> std::io::Result<Source> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if meta.len() == 0 {
        // 빈 파일은 mmap을 만들지 않고 None으로 표시한다 (Windows에서 0바이트 매핑 불가).
        return Ok(Source { mmap: None, owned: None });
    }
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(Source { mmap: Some(mmap), owned: None })
}

impl Source {
    /// 메모리 바이트로 Source를 만든다. 파일이 없는 문서(찾기 결과 행 추출)와
    /// 테스트가 함께 쓴다. `Vec`을 그대로 받아 소유권을 넘긴다 — 추출본은
    /// 수백 MB가 될 수 있으므로 복사를 한 번 더 하지 않는다.
    ///
    /// 빈 바이트도 허용된다: `bytes()`가 `owned`가 비면 빈 슬라이스를 주므로
    /// `open()`의 빈 파일 경로와 같은 상태가 된다(Windows 0바이트 mmap 문제와
    /// 무관하게 안전).
    pub fn from_bytes(bytes: Vec<u8>) -> Source {
        Source { mmap: None, owned: Some(bytes) }
    }

    fn bytes(&self) -> &[u8] {
        if let Some(m) = &self.mmap {
            &m[..]
        } else if let Some(v) = &self.owned {
            &v[..]
        } else {
            &[]
        }
    }

    pub fn len(&self) -> u64 {
        self.bytes().len() as u64
    }

    /// [start, end) 범위의 바이트 슬라이스. 범위는 파일 크기로 클램프된다.
    pub fn slice(&self, start: u64, end: u64) -> &[u8] {
        let b = self.bytes();
        let len = b.len() as u64;
        let s = start.min(len) as usize;
        let e = end.min(len).max(start.min(len)) as usize;
        &b[s..e]
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // 테스트마다 고유한 임시 파일명을 만든다. 내용 길이만으로 이름을 지으면
    // 같은 길이의 내용을 쓰는 병렬 테스트끼리 같은 파일을 truncate/mmap 하며
    // 경합(len()==0 등)이 나므로, 원자적 카운터로 유일성을 보장한다.
    fn temp_file_with(content: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("tv_test_{}_{}.txt", std::process::id(), id));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn opens_and_reports_len() {
        let p = temp_file_with(b"hello world");
        let src = open(&p).unwrap();
        assert_eq!(src.len(), 11);
    }

    #[test]
    fn slice_returns_correct_bytes() {
        let p = temp_file_with(b"hello world");
        let src = open(&p).unwrap();
        assert_eq!(src.slice(0, 5), b"hello");
        assert_eq!(src.slice(6, 11), b"world");
    }

    #[test]
    fn missing_file_returns_err() {
        assert!(open(std::path::Path::new("no_such_file_xyz.txt")).is_err());
    }

    #[test]
    fn empty_file_opens_without_error() {
        let mut path = std::env::temp_dir();
        path.push("tv_test_empty.txt");
        std::fs::File::create(&path).unwrap();
        let src = open(&path).unwrap();
        assert_eq!(src.len(), 0);
        assert_eq!(src.slice(0, 0), b"");
    }
}
