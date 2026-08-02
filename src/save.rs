use crate::edit::Newline;
use crate::parse::Encoding;
use encoding_rs::EUC_KR;
use std::io::Write;
use std::path::Path;

pub struct SaveOptions {
    pub enc: Encoding,
    pub bom: bool,
    pub newline: Newline,
}

/// 문자열을 대상 인코딩 바이트로 변환한다.
pub fn encode_bytes(s: &str, enc: Encoding) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => s.as_bytes().to_vec(),
        Encoding::Cp949 => {
            let (cow, _, _) = EUC_KR.encode(s);
            cow.into_owned()
        }
        Encoding::Utf16Le => {
            let mut out = Vec::with_capacity(s.len() * 2);
            for u in s.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        Encoding::Utf16Be => {
            let mut out = Vec::with_capacity(s.len() * 2);
            for u in s.encode_utf16() {
                out.extend_from_slice(&u.to_be_bytes());
            }
            out
        }
    }
}

fn bom_bytes(enc: Encoding) -> &'static [u8] {
    match enc {
        Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoding::Utf16Le => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
        Encoding::Cp949 => &[],
    }
}

/// lines를 대상 인코딩/개행으로 임시 파일에 쓴 뒤 path로 원자적 rename 한다.
pub fn write_file(
    path: &Path,
    lines: &[String],
    opts: &SaveOptions,
    progress: Option<&dyn Fn(usize)>,
) -> std::io::Result<()> {
    let tmp = {
        let mut t = path.to_path_buf();
        let name = t.file_name().map(|n| n.to_owned()).unwrap_or_default();
        let mut n = name;
        n.push(".tmp");
        t.set_file_name(n);
        t
    };
    // 개행도 대상 인코딩으로 재인코딩(UTF-16은 개행이 2바이트).
    let nl_encoded = encode_bytes(
        match opts.newline {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
        },
        opts.enc,
    );

    let result = (|| -> std::io::Result<()> {
        let f = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::new(f);
        if opts.bom {
            w.write_all(bom_bytes(opts.enc))?;
        }
        for (i, line) in lines.iter().enumerate() {
            w.write_all(&encode_bytes(line, opts.enc))?;
            w.write_all(&nl_encoded)?;
            if let Some(p) = progress {
                if i % 65536 == 0 {
                    p(i);
                }
            }
        }
        w.flush()?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// bytes를 임시 파일에 쓰고 path로 원자적 rename. `write_file`과 같은
/// 패턴이되 인코딩/개행 개념이 없다(바이너리는 바이트가 곧 전부다).
#[allow(dead_code)] // Task 7이 소비하면 제거
pub fn write_binary(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = {
        let mut t = path.to_path_buf();
        let name = t.file_name().map(|n| n.to_owned()).unwrap_or_default();
        let mut n = name;
        n.push(".tmp");
        t.set_file_name(n);
        t
    };
    match std::fs::write(&tmp, bytes) {
        Ok(()) => {
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let id = C.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_save_{}_{}_{}.txt", std::process::id(), id, name));
        p
    }

    fn v(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn write_utf8_lf() {
        let p = tmp_path("u8");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: false, newline: Newline::Lf };
        write_file(&p, &v(&["a", "b"]), &opts, None).unwrap();
        let got = std::fs::read(&p).unwrap();
        assert_eq!(got, b"a\nb\n");
    }

    #[test]
    fn write_crlf() {
        let p = tmp_path("crlf");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: false, newline: Newline::CrLf };
        write_file(&p, &v(&["a", "b"]), &opts, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nb\r\n");
    }

    #[test]
    fn write_utf8_bom() {
        let p = tmp_path("bom");
        let opts = SaveOptions { enc: Encoding::Utf8, bom: true, newline: Newline::Lf };
        write_file(&p, &v(&["x"]), &opts, None).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"\xEF\xBB\xBFx\n");
    }

    #[test]
    fn write_cp949_roundtrip() {
        let p = tmp_path("cp949");
        let opts = SaveOptions { enc: Encoding::Cp949, bom: false, newline: Newline::Lf };
        write_file(&p, &v(&["가나"]), &opts, None).unwrap();
        // CP949 "가나" = B0 A1 B3 AA
        assert_eq!(std::fs::read(&p).unwrap(), vec![0xB0, 0xA1, 0xB3, 0xAA, b'\n']);
    }

    #[test]
    fn write_utf16le_bom() {
        let p = tmp_path("u16le");
        let opts = SaveOptions { enc: Encoding::Utf16Le, bom: true, newline: Newline::Lf };
        write_file(&p, &v(&["A"]), &opts, None).unwrap();
        // BOM FF FE + 'A'(41 00) + '\n'(0A 00)
        assert_eq!(std::fs::read(&p).unwrap(), vec![0xFF, 0xFE, 0x41, 0x00, 0x0A, 0x00]);
    }

    #[test]
    fn encode_utf16be_char() {
        // 'A' in UTF-16BE = 00 41
        assert_eq!(encode_bytes("A", Encoding::Utf16Be), vec![0x00, 0x41]);
    }

    #[test]
    fn write_overwrites_existing_file() {
        // 주 사용 사례: 열었던 파일에 그대로 저장(덮어쓰기).
        // 임시파일 + rename이 기존 파일을 정확히 대체하는지, 이전 내용이
        // 남지 않는지(길이가 줄어드는 경우 포함) 확인한다.
        let p = tmp_path("overwrite");
        std::fs::write(&p, b"OLD CONTENT THAT IS QUITE LONG\n").unwrap();
        let opts = SaveOptions { enc: Encoding::Utf8, bom: false, newline: Newline::Lf };
        write_file(&p, &v(&["new"]), &opts, None).unwrap();
        // 새 내용만 남고 이전 내용의 잔재가 없어야 한다.
        assert_eq!(std::fs::read(&p).unwrap(), b"new\n");
        // 임시 파일이 남아 있으면 안 된다.
        let tmp = {
            let mut t = p.clone();
            let mut n = t.file_name().unwrap().to_owned();
            n.push(".tmp");
            t.set_file_name(n);
            t
        };
        assert!(!tmp.exists(), "임시 파일이 정리되어야 함");
    }

    #[test]
    fn write_binary_roundtrip_bitwise() {
        let p = tmp_path("bin");
        let data: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
        write_binary(&p, &data).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), data, "비트 단위 동일");
        // 덮어쓰기도 동작(임시파일 → rename 경로)
        write_binary(&p, &[0xDE, 0xAD]).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), vec![0xDE, 0xAD]);
        // 임시파일 잔존 없음
        let tmp = {
            let mut t = p.clone();
            let mut n = t.file_name().unwrap().to_owned();
            n.push(".tmp");
            t.set_file_name(n);
            t
        };
        assert!(!tmp.exists());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn write_binary_empty_ok() {
        let p = tmp_path("bin_empty");
        write_binary(&p, &[]).unwrap();
        assert_eq!(std::fs::read(&p).unwrap().len(), 0);
        std::fs::remove_file(&p).ok();
    }
}
