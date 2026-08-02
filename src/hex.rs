#![allow(dead_code)] // 아직 GUI가 소비하지 않는 항목들 — Task 9(마무리)에서 제거하고 재검증한다

//! 헥스 모드의 순수 로직 — 레이아웃 산술, 편집 버퍼, 바이트 검색.
//! egui 없음. `find.rs`/`edit.rs`/`convert.rs`와 같은 규율이다.

use memchr::memmem;

/// 한 행에 표시하는 바이트 수. 참고 UI(스크린샷) 관행.
pub const BYTES_PER_ROW: usize = 32;

/// 편집 진입(전체 메모리 로드) 시 확인 없이 허용하는 최대 크기.
pub const HEX_EDIT_CONFIRM_BYTES: u64 = 512 * 1024 * 1024;

/// 전체 행 수. 빈 파일도 한 행 — 캐럿을 둘 곳이 필요하다.
pub fn row_count(len: u64) -> u64 {
    if len == 0 {
        1
    } else {
        len.div_ceil(BYTES_PER_ROW as u64)
    }
}

/// 오프셋 컬럼 자릿수. 최소 6자리(참고 UI), 파일이 크면 마지막 오프셋이
/// 들어가는 만큼. 행마다 같은 폭이어야 컬럼이 흔들리지 않으므로 파일
/// 단위로 한 번 계산한다.
pub fn offset_width(len: u64) -> usize {
    let last = len.saturating_sub(1);
    let digits = if last == 0 { 1 } else { (64 - last.leading_zeros() as usize).div_ceil(4) };
    digits.max(6)
}

/// 행 시작 오프셋의 대문자 16진수, `width` 자리 0 패딩.
pub fn format_offset(row: u64, width: usize) -> String {
    format!("{:0width$X}", row * BYTES_PER_ROW as u64)
}

/// 문자 패널 표시: 출력 가능한 ASCII(0x20..=0x7E)만 그 글자, 밖은 '.'.
pub fn ascii_char(b: u8) -> char {
    if (0x20..=0x7E).contains(&b) {
        b as char
    } else {
        '.'
    }
}

/// 헥스 패널의 문자 컬럼 → (바이트 인덱스, 상위 니블). 바이트 i는
/// "4F " 세 문자 [3i, 3i+3)를 차지한다. 공백 클릭은 하위 니블 취급 —
/// 다음 바이트로 넘기면 바이트 사이 공백 클릭이 오른쪽으로 튀어 어색하다.
pub fn hex_click_byte(char_col: usize) -> Option<(usize, bool)> {
    let byte = char_col / 3;
    if byte >= BYTES_PER_ROW {
        return None;
    }
    // 세 문자 중 첫 칸("4"의 자리)만 상위 니블. 나머지 둘(하위 자리·공백)은
    // 하위 니블이다. (`% 3 == 0`과 같은 뜻 — clippy가 이 형태를 권한다.)
    Some((byte, char_col.is_multiple_of(3)))
}

/// 문자 패널의 문자 컬럼 → 바이트 인덱스(1문자 = 1바이트).
pub fn ascii_click_byte(char_col: usize) -> Option<usize> {
    (char_col < BYTES_PER_ROW).then_some(char_col)
}

/// 니블 타이핑: high면 상위 4비트, 아니면 하위 4비트를 바꾼 바이트.
pub fn apply_nibble(byte: u8, high: bool, nibble: u8) -> u8 {
    if high {
        (nibble << 4) | (byte & 0x0F)
    } else {
        (byte & 0xF0) | (nibble & 0x0F)
    }
}

/// 찾기 입력 해석: 공백을 무시한 16진수 짝수 자리 → 바이트열.
/// 홀수 자리·16진수 아닌 문자·빈 입력은 None(찾기 버튼 비활성 근거).
pub fn parse_hex_query(s: &str) -> Option<Vec<u8>> {
    let mut nibbles: Vec<u8> = Vec::new();
    for c in s.chars() {
        if c.is_whitespace() {
            continue;
        }
        nibbles.push(c.to_digit(16)? as u8);
    }
    if nibbles.is_empty() || nibbles.len() % 2 == 1 {
        return None;
    }
    Some(nibbles.chunks(2).map(|p| (p[0] << 4) | p[1]).collect())
}

/// from부터 전방 검색, 끝까지 없으면 처음부터(랩어라운드). memmem이라
/// GB급 mmap도 빠르다.
pub fn find_bytes(haystack: &[u8], needle: &[u8], from: u64) -> Option<u64> {
    if needle.is_empty() || haystack.is_empty() {
        return None;
    }
    let from = (from as usize).min(haystack.len());
    if let Some(p) = memmem::find(&haystack[from..], needle) {
        return Some((from + p) as u64);
    }
    memmem::find(haystack, needle).map(|p| p as u64)
}

/// 입력을 받는 패널. 클릭한 쪽이 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexPane {
    Hex,
    Ascii,
}

/// 헥스 문서의 화면·편집 상태. `Document.hex`에 담긴다.
pub struct HexState {
    /// None = 뷰(mmap), Some = 편집(메모리 전체 로드).
    /// 텍스트 모드의 `Document.edit` 승격과 같은 구조다.
    pub edit: Option<HexEditBuffer>,
    /// 캐럿: (바이트 오프셋, 상위 니블인가). 문자 패널에선 니블 무시.
    pub caret: (u64, bool),
    /// 선택 [anchor, caret) — 바이트 오프셋, 방향 무관 저장.
    pub sel: Option<(u64, u64)>,
    pub pane: HexPane,
    /// Insert 키로 토글. true면 타이핑이 삽입, false면 덮어쓰기.
    pub insert_mode: bool,
    /// 찾기 입력을 16진수로 해석할지(false면 UTF-8 텍스트).
    pub find_hex: bool,
    /// 마지막 매치 (오프셋, 길이). 하이라이트와 다음 찾기 기준.
    pub last_match: Option<(u64, usize)>,
    /// 512MB 초과 파일의 편집 진입 확인 대기 중인가.
    pub confirm_load: bool,
}

impl HexState {
    pub fn new() -> Self {
        HexState {
            edit: None,
            caret: (0, true),
            sel: None,
            pane: HexPane::Hex,
            insert_mode: false,
            find_hex: true,
            last_match: None,
            confirm_load: false,
        }
    }
}

/// 편집 연산 기록. old/new가 Vec인 이유: 문자 패널에서 한글 한 글자는
/// UTF-8 3바이트 덮어쓰기라, 한 글자 입력이 undo 한 번으로 돌아가야 한다.
#[derive(Debug, Clone, PartialEq)]
enum HexOp {
    Overwrite { offset: u64, old: Vec<u8>, new: Vec<u8> },
    Insert { offset: u64, bytes: Vec<u8> },
    Delete { offset: u64, bytes: Vec<u8> },
}

/// 메모리 전체 로드 편집 버퍼. 저장은 `save::write_binary`가 통째로 쓴다.
pub struct HexEditBuffer {
    pub bytes: Vec<u8>,
    pub dirty: bool,
    undo: Vec<HexOp>,
    redo: Vec<HexOp>,
}

impl HexEditBuffer {
    pub fn new(bytes: Vec<u8>) -> Self {
        HexEditBuffer { bytes, dirty: false, undo: Vec::new(), redo: Vec::new() }
    }

    fn push_op(&mut self, op: HexOp) {
        self.undo.push(op);
        self.redo.clear();
        self.dirty = true;
    }

    /// offset부터 new로 덮어쓴다. 파일 끝을 넘으면 넘치는 만큼 이어붙인다
    /// (스펙 — 문자 패널에서 마지막 바이트에 멀티바이트 글자를 칠 때).
    pub fn overwrite(&mut self, offset: u64, new: &[u8]) {
        if new.is_empty() {
            return;
        }
        let o = (offset as usize).min(self.bytes.len());
        let end = (o + new.len()).min(self.bytes.len());
        let old = self.bytes[o..end].to_vec();
        self.bytes[o..end].copy_from_slice(&new[..end - o]);
        if o + new.len() > self.bytes.len() {
            self.bytes.extend_from_slice(&new[end - o..]);
        }
        self.push_op(HexOp::Overwrite { offset: o as u64, old, new: new.to_vec() });
    }

    pub fn insert(&mut self, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let o = (offset as usize).min(self.bytes.len());
        self.bytes.splice(o..o, bytes.iter().copied());
        self.push_op(HexOp::Insert { offset: o as u64, bytes: bytes.to_vec() });
    }

    /// [start, end) 삭제. 범위는 버퍼 길이로 클램프, 빈 범위는 no-op.
    pub fn delete_range(&mut self, start: u64, end: u64) {
        let s = (start as usize).min(self.bytes.len());
        let e = (end as usize).min(self.bytes.len());
        if s >= e {
            return;
        }
        let removed: Vec<u8> = self.bytes.splice(s..e, std::iter::empty()).collect();
        self.push_op(HexOp::Delete { offset: s as u64, bytes: removed });
    }

    /// 되돌리고 캐럿을 둘 오프셋을 준다. 스택이 비면 None.
    pub fn undo(&mut self) -> Option<u64> {
        let op = self.undo.pop()?;
        let pos = match &op {
            HexOp::Overwrite { offset, old, new } => {
                let o = *offset as usize;
                // new가 적용된 구간 [o, o+new.len())을 old로 되돌린다.
                // old가 짧으면(끝 확장이었으면) 길이도 함께 줄어든다.
                self.bytes.splice(o..o + new.len(), old.iter().copied());
                *offset
            }
            HexOp::Insert { offset, bytes } => {
                let o = *offset as usize;
                self.bytes.splice(o..o + bytes.len(), std::iter::empty());
                *offset
            }
            HexOp::Delete { offset, bytes } => {
                let o = *offset as usize;
                self.bytes.splice(o..o, bytes.iter().copied());
                *offset
            }
        };
        self.redo.push(op);
        self.dirty = true;
        Some(pos)
    }

    pub fn redo(&mut self) -> Option<u64> {
        let op = self.redo.pop()?;
        let pos = match &op {
            HexOp::Overwrite { offset, old, new } => {
                let o = *offset as usize;
                self.bytes.splice(o..o + old.len(), new.iter().copied());
                *offset
            }
            HexOp::Insert { offset, bytes } => {
                let o = *offset as usize;
                self.bytes.splice(o..o, bytes.iter().copied());
                *offset
            }
            HexOp::Delete { offset, bytes } => {
                let o = *offset as usize;
                self.bytes.splice(o..o + bytes.len(), std::iter::empty());
                *offset
            }
        };
        self.undo.push(op);
        self.dirty = true;
        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 레이아웃 산술 ----

    #[test]
    fn row_count_boundaries() {
        assert_eq!(row_count(0), 1, "빈 파일도 한 행(캐럿 둘 곳)이 필요하다");
        assert_eq!(row_count(1), 1);
        assert_eq!(row_count(31), 1);
        assert_eq!(row_count(32), 1);
        assert_eq!(row_count(33), 2);
        assert_eq!(row_count(64), 2);
        assert_eq!(row_count(65), 3);
    }

    #[test]
    fn offset_width_grows_past_16mb() {
        assert_eq!(offset_width(0), 6);
        assert_eq!(offset_width(0xFF_FFFF), 6);          // 16MB-1 → 6자리로 충분
        assert_eq!(offset_width(0x100_0001), 7);          // 마지막 행 오프셋이 7자리
        assert_eq!(offset_width(0x1_0000_0001), 9);
    }

    #[test]
    fn format_offset_matches_reference_ui() {
        // 스크린샷 관행: 행 시작 오프셋의 대문자 16진수, 최소 6자리.
        assert_eq!(format_offset(0, 6), "000000");
        assert_eq!(format_offset(1, 6), "000020");
        assert_eq!(format_offset(2, 6), "000040");
        assert_eq!(format_offset(9, 6), "000120");
        assert_eq!(format_offset(0x100_0000 / 32, 7), "1000000");
    }

    #[test]
    fn ascii_char_printable_range_only() {
        assert_eq!(ascii_char(0x1F), '.');
        assert_eq!(ascii_char(0x20), ' ');
        assert_eq!(ascii_char(b'A'), 'A');
        assert_eq!(ascii_char(0x7E), '~');
        assert_eq!(ascii_char(0x7F), '.');
        assert_eq!(ascii_char(0x80), '.');
        assert_eq!(ascii_char(0xFF), '.');
        assert_eq!(ascii_char(0x00), '.');
    }

    #[test]
    fn apply_nibble_high_and_low() {
        assert_eq!(apply_nibble(0x00, true, 0xA), 0xA0);
        assert_eq!(apply_nibble(0x00, false, 0xA), 0x0A);
        assert_eq!(apply_nibble(0xFF, true, 0x0), 0x0F);
        assert_eq!(apply_nibble(0xFF, false, 0x0), 0xF0);
        assert_eq!(apply_nibble(0x12, true, 0x3), 0x32);
        assert_eq!(apply_nibble(0x12, false, 0x3), 0x13);
    }

    // ---- 클릭 산술 ----

    #[test]
    fn hex_click_byte_maps_columns() {
        // 바이트 i는 문자 컬럼 [3i, 3i+3) — "4F " 두 자리 + 공백.
        assert_eq!(hex_click_byte(0), Some((0, true)));   // 상위 니블
        assert_eq!(hex_click_byte(1), Some((0, false)));  // 하위 니블
        assert_eq!(hex_click_byte(2), Some((0, false)));  // 공백은 그 바이트 하위로
        assert_eq!(hex_click_byte(3), Some((1, true)));
        assert_eq!(hex_click_byte(95), Some((31, false))); // 마지막 바이트 끝
        assert_eq!(hex_click_byte(96), None, "행 폭 밖");
    }

    #[test]
    fn ascii_click_byte_maps_columns() {
        assert_eq!(ascii_click_byte(0), Some(0));
        assert_eq!(ascii_click_byte(31), Some(31));
        assert_eq!(ascii_click_byte(32), None);
    }

    // ---- 찾기 입력 해석 ----

    #[test]
    fn parse_hex_query_accepts_spaced_and_packed() {
        assert_eq!(parse_hex_query("53 51 4C"), Some(vec![0x53, 0x51, 0x4C]));
        assert_eq!(parse_hex_query("53514c"), Some(vec![0x53, 0x51, 0x4C]));
        assert_eq!(parse_hex_query("  53  51 "), Some(vec![0x53, 0x51]));
        assert_eq!(parse_hex_query("ab CD ef"), Some(vec![0xAB, 0xCD, 0xEF]));
    }

    #[test]
    fn parse_hex_query_rejects_bad_input() {
        assert_eq!(parse_hex_query(""), None, "빈 입력은 찾을 것이 없다");
        assert_eq!(parse_hex_query("   "), None);
        assert_eq!(parse_hex_query("5"), None, "홀수 자리");
        assert_eq!(parse_hex_query("53 5"), None);
        assert_eq!(parse_hex_query("5G"), None, "16진수 아닌 문자");
        assert_eq!(parse_hex_query("한글"), None);
    }

    // ---- 바이트 검색 ----

    #[test]
    fn find_bytes_forward_then_wraps() {
        let hay = b"abcXabcX";
        assert_eq!(find_bytes(hay, b"abc", 0), Some(0));
        assert_eq!(find_bytes(hay, b"abc", 1), Some(4));
        assert_eq!(find_bytes(hay, b"abc", 5), Some(0), "끝까지 없으면 처음부터");
        assert_eq!(find_bytes(hay, b"zzz", 0), None);
        assert_eq!(find_bytes(hay, b"", 0), None, "빈 바늘은 무의미");
        assert_eq!(find_bytes(b"", b"a", 0), None);
        assert_eq!(find_bytes(hay, b"abc", 999), Some(0), "범위 밖 from은 랩");
    }

    // ---- 편집 버퍼 ----

    #[test]
    fn overwrite_replaces_and_sets_dirty() {
        let mut b = HexEditBuffer::new(vec![1, 2, 3, 4]);
        assert!(!b.dirty);
        b.overwrite(1, &[9, 8]);
        assert_eq!(b.bytes, vec![1, 9, 8, 4]);
        assert!(b.dirty);
    }

    #[test]
    fn overwrite_past_end_extends() {
        // 스펙: 파일 끝을 넘는 덮어쓰기는 넘치는 만큼 이어붙인다.
        let mut b = HexEditBuffer::new(vec![1, 2]);
        b.overwrite(1, &[7, 8, 9]);
        assert_eq!(b.bytes, vec![1, 7, 8, 9]);
        assert_eq!(b.undo(), Some(1));
        assert_eq!(b.bytes, vec![1, 2], "undo가 늘어난 길이도 되돌린다");
    }

    #[test]
    fn insert_and_delete_roundtrip_via_undo() {
        let mut b = HexEditBuffer::new(vec![1, 2, 3]);
        b.insert(1, &[9, 9]);
        assert_eq!(b.bytes, vec![1, 9, 9, 2, 3]);
        b.delete_range(2, 4);
        assert_eq!(b.bytes, vec![1, 9, 3]);
        assert_eq!(b.undo(), Some(2), "delete 취소 — 지운 자리로 캐럿");
        assert_eq!(b.bytes, vec![1, 9, 9, 2, 3]);
        assert_eq!(b.undo(), Some(1), "insert 취소");
        assert_eq!(b.bytes, vec![1, 2, 3]);
        assert_eq!(b.undo(), None, "더 되돌릴 것 없음");
    }

    #[test]
    fn redo_reapplies_and_new_edit_clears_redo() {
        let mut b = HexEditBuffer::new(vec![1, 2, 3]);
        b.overwrite(0, &[7]);
        b.undo();
        assert_eq!(b.bytes, vec![1, 2, 3]);
        assert_eq!(b.redo(), Some(0));
        assert_eq!(b.bytes, vec![7, 2, 3]);
        b.undo();
        b.insert(3, &[5]); // 새 편집이 redo 스택을 지운다
        assert_eq!(b.redo(), None);
    }

    #[test]
    fn delete_range_clamps_to_len_and_ignores_empty() {
        let mut b = HexEditBuffer::new(vec![1, 2, 3]);
        b.delete_range(2, 999); // 끝 클램프
        assert_eq!(b.bytes, vec![1, 2]);
        let dirty_before = b.dirty;
        b.delete_range(1, 1); // 빈 범위 — no-op, undo에 안 쌓임
        b.delete_range(5, 9); // 전부 범위 밖 — no-op
        assert_eq!(b.bytes, vec![1, 2]);
        assert_eq!(b.dirty, dirty_before);
        assert_eq!(b.undo(), Some(2), "실제 삭제 하나만 쌓였다");
        assert_eq!(b.undo(), None);
    }

    #[test]
    fn hex_state_defaults() {
        let h = HexState::new();
        assert!(h.edit.is_none());
        assert_eq!(h.caret, (0, true));
        assert!(h.sel.is_none());
        assert!(matches!(h.pane, HexPane::Hex));
        assert!(!h.insert_mode);
        assert!(h.find_hex, "헥스 문서의 찾기 기본 해석은 16진수");
        assert!(h.last_match.is_none());
        assert!(!h.confirm_load);
    }
}
