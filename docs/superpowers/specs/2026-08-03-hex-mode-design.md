# 바이너리 헥스 모드 설계

**작성일:** 2026-08-03
**브랜치:** `master`

## 목표

텍스트로 해석되지 않는 파일을 열 때 (1) 인코딩을 강제 지정해 텍스트로 열거나
(2) 바이너리(헥스) 모드로 열 수 있게 한다. 헥스 모드는 오프셋 + 16진수 +
문자 패널 레이아웃으로 표시하고, 바이트 단위 편집(덮어쓰기·삽입·삭제)과
저장, 16진수/텍스트 찾기를 지원한다.

## 배경 — 현재는 "실패"가 없다

`detect_encoding`(`parse.rs:13`)은 BOM → UTF-8 → **CP949 폴백** 순서다.
CP949 폴백은 어떤 바이트열이든 받아들이므로 감지가 실패하는 경우가 없고,
SQLite/GPKG 같은 바이너리도 깨진 CP949 텍스트로 열린다. "안 풀리는 파일"
판정 자체를 새로 만들어야 한다.

## 감지

`parse.rs`에 추가:

```rust
pub enum TextDetection {
    Text(Encoding),
    Binary,
}

/// 파일 앞부분 샘플로 텍스트/바이너리를 판정한다.
pub fn detect_text(head: &[u8]) -> TextDetection;
```

판정 순서:

1. **BOM이 있으면 무조건 텍스트** (UTF-8/UTF-16LE/BE). UTF-16은 NUL
   바이트투성이므로 NUL 검사보다 먼저 와야 한다.
2. **NUL(0x00)이 샘플에 있으면 바이너리.** 텍스트 파일에는 NUL이 없고,
   실행 파일·DB·이미지 등 대부분의 바이너리는 앞부분에 NUL이 있다.
   (BOM 없는 UTF-16은 이 규칙에 걸리지만, 현재 감지도 BOM 없는 UTF-16을
   텍스트로 못 읽으므로 회귀가 아니다 — 다이얼로그에서 UTF-16을 강제
   지정하면 열린다. 오히려 개선이다.)
3. 나머지는 `detect_encoding` 결과로 디코드해서 **대체문자(U+FFFD)가
   디코드 결과 문자 수의 5% 초과면 바이너리**, 이하면 그 인코딩의 텍스트.
   5%는 "몇 바이트 깨진 텍스트 파일"(지금도 열리는 파일)은 계속 텍스트로
   열고, 무작위 바이트열(CP949 디코드에서 FFFD가 대량 발생)은 걸러내는
   관대한 값이다.

기존 `detect_encoding`은 그대로 두고 `detect_text`가 내부에서 부른다 —
인덱서 등 다른 호출처는 영향받지 않는다.

## 열기 다이얼로그

`open_path`가 `Binary` 판정을 받으면 문서를 만들지 않고 App에 보류 상태를
남긴다:

```rust
/// 바이너리로 판정돼 사용자의 열기 방식 선택을 기다리는 파일.
pub pending_binary_open: Option<PendingBinaryOpen>,

pub struct PendingBinaryOpen {
    pub path: std::path::PathBuf,
    /// 다이얼로그의 "텍스트로 열기"용 인코딩 선택값.
    pub enc: Encoding,
}
```

`egui::Window` 다이얼로그 (Sort/Convert 다이얼로그와 같은 패턴):

```
┌─ 텍스트가 아닌 파일 ───────────────────────────┐
│ 이 파일은 텍스트로 해석되지 않습니다.           │
│ (경로 표시)                                    │
│                                                │
│ [바이너리(헥스)로 열기]                         │
│                                                │
│ 또는 인코딩을 지정해 텍스트로 열기:             │
│   (UTF-8 ▾)  [텍스트로 열기]                   │
│                                                │
│                                    [취소]      │
└────────────────────────────────────────────────┘
```

- **바이너리(헥스)로 열기** — 기본 경로. 헥스 문서 생성.
- **텍스트로 열기** — 고른 인코딩(UTF-8/CP949/UTF-16LE/UTF-16BE)으로 기존
  텍스트 열기 경로를 그대로 탄다(감지만 건너뛰고 인코딩을 주입). 깨진
  글자(�)는 감수한다.
- **취소** — 아무것도 열지 않는다.

`open_path`는 세 갈래로 나뉜다: 판정을 하는 기존 진입점, 인코딩을 주입받는
`open_path_as_text(path, enc)`, 헥스로 여는 `open_path_hex(path)`. 다이얼로그
버튼이 뒤의 둘을 부른다.

## 아키텍처

### 새 모듈: `src/hex.rs`

순수 로직(egui 없음). `find.rs`/`edit.rs`/`convert.rs`와 같은 규율이다.

```rust
/// 한 행에 표시하는 바이트 수. 스크린샷(참고 UI) 관행을 따른다.
pub const BYTES_PER_ROW: usize = 32;

/// 편집 진입(전체 메모리 로드) 시 확인 없이 허용하는 최대 크기.
pub const HEX_EDIT_CONFIRM_BYTES: u64 = 512 * 1024 * 1024;

pub fn row_count(len: u64) -> u64;                 // ⌈len/32⌉, 0바이트면 1
pub fn format_offset(row: u64) -> String;          // "000000" 6자리 이상 16진수
pub fn ascii_char(b: u8) -> char;                  // 0x20..=0x7E는 그 글자, 밖은 '.'

/// 니블 타이핑: high면 상위 4비트, 아니면 하위 4비트를 바꾼 바이트.
pub fn apply_nibble(byte: u8, high: bool, nibble: u8) -> u8;

/// 찾기 입력 해석: "53 51 4C" / "53514c" → Some(bytes). 홀수 자리·잘못된
/// 문자는 None(찾기 버튼 비활성 근거).
pub fn parse_hex_query(s: &str) -> Option<Vec<u8>>;

/// memchr::memmem 전방 검색. from부터, 끝까지 없으면 처음부터 랩어라운드.
pub fn find_bytes(haystack: &[u8], needle: &[u8], from: u64) -> Option<u64>;

pub struct HexEditBuffer {
    pub bytes: Vec<u8>,
    pub dirty: bool,
    undo: Vec<HexOp>,
    redo: Vec<HexOp>,
}

enum HexOp {
    /// old/new가 Vec인 이유: 문자 패널에서 한글 한 글자는 UTF-8 3바이트
    /// 덮어쓰기라, 한 글자 입력이 undo 한 번으로 돌아가야 한다.
    /// 파일 끝을 넘는 덮어쓰기는 넘치는 만큼 이어붙이며(new.len() > old.len()),
    /// undo가 old 길이로 되돌린다.
    Overwrite { offset: u64, old: Vec<u8>, new: Vec<u8> },
    Insert { offset: u64, bytes: Vec<u8> },
    Delete { offset: u64, bytes: Vec<u8> },
}

impl HexEditBuffer {
    pub fn overwrite(&mut self, offset: u64, new: &[u8]);
    pub fn insert(&mut self, offset: u64, bytes: &[u8]);
    pub fn delete_range(&mut self, start: u64, end: u64); // [start, end)
    pub fn undo(&mut self) -> Option<u64>;  // 되돌린 위치(캐럿 이동용)
    pub fn redo(&mut self) -> Option<u64>;
}
```

### Document 확장

```rust
/// Some이면 이 문서는 헥스 모드다. 텍스트 문서와 배타적이며, 텍스트 관련
/// 필드(sep, index, edit, ...)는 헥스 문서에서 쓰이지 않는다.
pub hex: Option<HexState>,

pub struct HexState {
    /// None = 뷰(mmap), Some = 편집(메모리 전체 로드). 텍스트 모드의
    /// `Document.edit` 승격과 같은 구조다.
    pub edit: Option<hex::HexEditBuffer>,
    /// 캐럿: (바이트 오프셋, 상위 니블인가). 문자 패널에선 니블 무시.
    pub caret: (u64, bool),
    /// 선택 범위 [anchor, caret) — 바이트 단위, 방향 무관 저장.
    pub sel: Option<(u64, u64)>,
    /// 입력을 받는 패널. 클릭한 쪽이 된다.
    pub pane: HexPane,       // Hex | Ascii
    /// Insert 키로 토글. true면 타이핑이 삽입, false면 덮어쓰기.
    pub insert_mode: bool,
    /// 찾기 입력을 16진수로 해석할지(false면 UTF-8 텍스트).
    pub find_hex: bool,
    /// 마지막 매치 (오프셋, 길이). 하이라이트와 다음 찾기 기준.
    pub last_match: Option<(u64, usize)>,
}
```

헥스 문서 생성 시: `sep = SeparatorMode::None`, `has_header = false`,
**인덱서를 띄우지 않는다**(줄 개념이 없다). `LineIndex`는 빈 것을 넣는다.

### 렌더

`app.rs`의 본문 렌더 분기에 헥스 경로 추가. 기존 텍스트 뷰와 같은
`TableBuilder` 가상 스크롤 — 행 수 = `row_count(len)`, 보이는 행만:

- 바이트 출처: 편집 중이면 `edit.bytes`, 아니면 mmap `source.slice`.
- 한 행 = 오프셋 컬럼 + 헥스 컬럼(바이트당 3문자 폭 고정) + 문자
  컬럼(바이트당 1문자). 모노스페이스 고정폭이므로 클릭 x좌표 → 바이트
  인덱스 역산이 산술로 된다(갤리 히트테스트 불필요).
- 헥스/문자 패널 모두 `LayoutJob` 섹션으로 바이트별 색을 준다: 선택 배경,
  매치 하이라이트, 캐럿. 수정된 바이트 강조는 undo 스택에서 위치 집합을
  뽑지 않는다 — 삽입/삭제로 오프셋이 밀리면 추적이 복잡해지므로 **범위
  밖**으로 미룬다(아래 YAGNI).

## 헥스 뷰 (기본 상태)

- mmap이므로 크기 무관 즉시 열린다. 캐럿 이동·클릭·선택·찾기는 뷰에서도
  된다(모두 읽기 전용 조작).
- 키: 방향키/PgUp/PgDn/Home/End 캐럿 이동, Shift+이동 선택 확장,
  Ctrl+C는 선택 구간을 16진수 문자열("53 51 4C")로 클립보드에 복사.

## 헥스 편집 (첫 편집 시 승격)

뷰 상태에서 **편집 조작**(니블/문자 타이핑, Delete/Backspace, 붙여넣기)이
오면:

1. 파일 크기 ≤ `HEX_EDIT_CONFIRM_BYTES`(512MB)면 즉시 전체를 `Vec<u8>`로
   로드하고 그 조작을 적용한다.
2. 초과면 확인 다이얼로그("파일 전체를 메모리에 올립니다. 계속?")를 띄우고,
   확인 후 로드한다(기존 `pending_column_op` 확인 패턴과 같다). 이때 원래
   조작은 버려진다 — 로드가 끝난 뒤 다시 입력하면 된다.

편집 조작:

- **덮어쓰기(기본)**: 헥스 패널에서 16진수 문자(0-9, a-f) 타이핑 →
  `apply_nibble`로 캐럿 니블 교체, 캐럿 한 니블 전진. 문자 패널에서 글자
  타이핑 → 그 글자의 **UTF-8 바이트열**로 캐럿부터 덮어쓴다(ASCII는 1바이트,
  한글은 3바이트 덮어쓰기 — IME 확정 글자도 같은 경로).
- **삽입 모드**: Insert 키 토글. 타이핑 위치에 바이트가 새로 끼어든다
  (헥스 패널은 상위 니블 입력 시 새 바이트 삽입 후 하위 니블 대기).
- **삭제**: 선택이 있으면 선택 구간 삭제, 없으면 Delete = 캐럿 바이트,
  Backspace = 캐럿 앞 바이트.
- **붙여넣기(Ctrl+V)**: 헥스 패널이면 클립보드를 `parse_hex_query`로 해석해
  그 바이트열을(해석 불가면 무시), 문자 패널이면 UTF-8 바이트열을 캐럿에
  삽입(삽입 모드 무관 — 붙여넣기는 관행상 삽입).
- **Ctrl+Z / Ctrl+Y**: `HexOp` 단위 undo/redo. 되돌린 위치로 캐럿 이동.

문서 탭의 dirty 표시·닫기 확인은 텍스트 편집과 같은 규칙을 탄다
(`edit.dirty` 대신 `hex.edit.dirty`를 본다).

## 저장

`save.rs`에 추가:

```rust
/// bytes를 임시 파일에 쓰고 path로 원자적 rename. write_file과 같은
/// tmp → rename 패턴.
pub fn write_binary(path: &Path, bytes: &[u8]) -> std::io::Result<()>;
```

- **Ctrl+S / Save**: 헥스 편집 버퍼가 있으면 `write_binary` 후 dirty 해제.
  편집 전(뷰)이면 바꾼 게 없으므로 비활성.
- **Save As**: 편집 중이면 `write_binary`를 새 경로로. 뷰 상태면 비활성
  (단순 파일 복사는 에디터의 일이 아니다). 저장 후 `doc.path` 갱신은
  텍스트와 같은 규칙.
- 저장 다이얼로그에서 **인코딩/BOM/개행 콤보를 숨긴다** — 바이너리에
  무의미하고, 보이면 "선택하면 뭔가 변환되나?"라는 오해를 만든다. 확장자
  필터는 "All files"만.

## 찾기

헥스 문서에서 Ctrl+F로 기존 찾기 패널을 재사용하되 내용을 갈아 끼운다:

- 입력 해석 토글: **16진수**(기본) / 텍스트. 16진수면 `parse_hex_query`,
  텍스트면 UTF-8 바이트열.
- 검색 대상: 편집 중이면 `edit.bytes`, 아니면 mmap 전체 슬라이스.
  `memchr::memmem`이라 GB급도 빠르다. 뷰 상태의 mmap 검색은 파일 크기와
  무관하게 즉시다(SSD 순차 읽기).
- Enter/F3 = 다음 찾기(`last_match` 다음 오프셋부터, 끝나면 랩어라운드).
- 매치는 `last_match`에 저장, 헥스·문자 패널 양쪽 하이라이트 + 그 행으로
  스크롤(`pending_scroll_row` 재사용).
- 바꾸기·Find All 하이라이트는 범위 밖(YAGNI).

## 기존 기능 게이트

헥스 문서(`doc.hex.is_some()`)에서 숨기거나 비활성:

- 툴바: Delimiter 드롭다운, 헤더 체크박스, 인코딩 드롭다운(표시 전용으로
  "Binary" 라벨), Edit mode 토글(헥스는 자동 승격이 대신한다)
- 메뉴: Sort, Convert Delimiter, Row & Column Numbers, 오류 행 창
- 줄끝 기호·행 번호 거터 등 텍스트 전용 렌더 요소
- Tab 문자 삽입 경로(`wants_tab_character`)는 `d.edit`(텍스트 편집)만 보므로
  헥스 문서에서는 이미 false — 회귀 테스트만 추가

상태줄(있는 위치)에 캐럿 오프셋을 `0x1A2B (6699)` 형식으로 표시한다.

## 테스트

전부 기존 방식대로: 순수 함수 단위 테스트 + 변이(mutation) 확인.

- **감지**: BOM 3종 → 텍스트, NUL 포함 → 바이너리, SQLite 헤더 실샘플 →
  바이너리, 정상 UTF-8/CP949 → 텍스트, FFFD 5% 경계(4%는 텍스트, 6%는
  바이너리), 빈 파일 → 텍스트.
- **hex.rs 순수 함수**: `row_count` 경계(0, 31, 32, 33바이트),
  `format_offset` 자릿수, `ascii_char` 경계(0x1F/0x20/0x7E/0x7F),
  `apply_nibble` 상/하위, `parse_hex_query`(공백 섞임·홀수 자리·잘못된
  문자·빈 문자열), `find_bytes` 랩어라운드·미발견.
- **HexEditBuffer**: 덮어쓰기/삽입/삭제 각각 undo → 원본 복원, redo 재적용,
  편집 후 redo 스택 클리어, dirty 전이, 삭제 범위 경계([start,end) 반개구간,
  파일 끝).
- **왕복**: bytes → `write_binary` → 읽기 → 비트 동일성. 임시파일 잔존 없음.
- **다이얼로그 상태 전이**: 바이너리 판정 → `pending_binary_open` 설정,
  각 버튼 → 올바른 열기 경로 호출/취소, 순수 함수 가드로 뽑아 테스트.
- **게이트**: 헥스 문서에서 Sort/Convert/Tab 삽입이 비활성인지 — 판정을
  자유 함수로 뽑아 프로덕션과 테스트가 같은 함수를 부른다(이 코드베이스의
  확립된 규율).
- **클릭 역산**: x좌표 → 바이트 인덱스 산술(헥스 패널 3문자 폭, 문자 패널
  1문자 폭) 경계값.

## 범위 밖 (YAGNI)

- **수정된 바이트 색 표시** — 삽입/삭제가 오프셋을 밀면 "원본과 다른 위치"
  추적이 diff 문제가 된다. 덮어쓰기만이라면 쉬웠으나 삽입/삭제를 받은 대가.
  필요해지면 저장 후 리로드 비교로 추가.
- **바꾸기(Replace)** — 찾기만. 필요해지면 추가.
- **Find All 하이라이트** — 단일 매치 하이라이트만.
- **바이트/행 수 조절(16/32 토글)** — 32 고정.
- **512MB 초과 파일의 덮어쓰기 전용 편집(제자리 패치)** — 전체 로드 확인
  다이얼로그로 갈음. 실사용에서 수 GB 바이너리 편집 요구가 나오면 그때.
- **텍스트 문서 ↔ 헥스 문서 전환** — 열 때 정해지면 그대로. 같은 파일을
  헥스로 다시 보고 싶으면 다시 열면 된다.
- **BOM 없는 UTF-16 자동 감지** — 다이얼로그에서 수동 지정으로 해결.
