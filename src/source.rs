use memmap2::Mmap;
use std::path::Path;

/// mmap으로 매핑된 읽기 전용 파일. 파일 크기와 무관하게 즉시 매핑된다.
///
/// Windows에서는 길이 0인 실제 파일을 `Mmap::map`으로도, `MmapOptions::map_anon`으로도
/// 매핑할 수 없다(둘 다 `ERROR_INVALID_PARAMETER`로 실패하는 것을 확인). 따라서 빈 파일은
/// `mmap: None`으로 표현하고, `len`/`slice`/`as_bytes`가 이를 빈 슬라이스로 처리한다.
pub struct Source {
    mmap: Option<Mmap>,
}

/// 파일을 열어 메모리 매핑한다. 빈 파일도 허용.
pub fn open(path: &Path) -> std::io::Result<Source> {
    let file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if meta.len() == 0 {
        // 빈 파일은 mmap을 만들지 않고 None으로 표시한다 (Windows에서 0바이트 매핑 불가).
        return Ok(Source { mmap: None });
    }
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(Source { mmap: Some(mmap) })
}

impl Source {
    pub fn len(&self) -> u64 {
        match &self.mmap {
            Some(m) => m.len() as u64,
            None => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// [start, end) 범위의 바이트 슬라이스. 범위는 파일 크기로 클램프된다.
    pub fn slice(&self, start: u64, end: u64) -> &[u8] {
        let Some(m) = &self.mmap else {
            return &[];
        };
        let len = m.len() as u64;
        let s = start.min(len) as usize;
        let e = end.min(len).max(start.min(len)) as usize;
        &m[s..e]
    }

    pub fn as_bytes(&self) -> &[u8] {
        match &self.mmap {
            Some(m) => &m[..],
            None => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file_with(content: &[u8]) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("tv_test_{}.txt", content.len()));
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
