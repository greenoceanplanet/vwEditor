# 편집 모드 + 저장/다른 이름으로 저장(인코딩 변환) 설계

> 뷰 전용(mmap)이던 앱에 편집 기능을 추가한다. 편집 모드 진입 시 파일 전체를
> 줄 배열로 RAM에 로드하고, 텍스트 모드는 자유 텍스트 편집, 세퍼레이터 모드는
> 셀 단위 편집을 제공한다. 저장 시 대상 인코딩으로 재인코딩하며 스트리밍한다.

**작성일:** 2026-07-23
**목표:** 큰 파일도 편집 가능하게 하되, 뷰 전용 즉시 열기 강점은 유지. 저장·다른
이름으로 저장 시 인코딩(UTF-8/CP949/UTF-16LE/BE) 변환 지원.

---

## 결정 사항 (확정)

- **편집 진입 시 전부 RAM 로드**: 편집 모드로 들어갈 때 파일 전체를 줄 단위
  `Vec<String>`(= `EditBuffer.lines`)로 읽어 인메모리 모델로 전환한다. 뷰 전용
  경로(mmap + LineIndex)는 그대로 두고, 편집 중에는 `EditBuffer`가 진실의 원천이
  된다. (근거: emeditor도 편집 시 행수 비례 메모리 사용. "램 절약 과하게 마라"
  이전 피드백과 일치. piece table/mmap 조각은 현재 요구엔 과임 — YAGNI.)
- **인메모리 모델 = 줄 배열 `Vec<String>`**: 한 줄 = 한 논리 행. 이 앱의 모든
  것(행 번호, 셀, 정렬 permutation, 표/텍스트 렌더)이 이미 "줄" 단위라 자연스럽게
  맞는다. rope 크레이트는 셀/행/정렬과 임피던스 불일치라 채택 안 함.
- **두 모드, 다른 편집 방식**:
  - 텍스트 모드(`SeparatorMode::None`): 자유 텍스트 편집. 문자 커서·선택·삽입·삭제,
    Enter=줄 분할, 줄 경계 Backspace/Delete=줄 병합. 줄에 걸친 범위 드래그 선택.
  - 세퍼레이터 모드(`SeparatorMode::Char`): 셀 단위 편집. 셀 하나를 편집하면 그
    줄을 필드로 쪼개→해당 필드 교체→구분자로 재조립. 셀 사각 범위(행×열) 드래그.
- **드래그 선택 + 통삭제/통붙여넣기 + 우클릭 메뉴**: 두 모드 모두 지원.
- **정렬과 편집 독립 공존**: 편집은 논리 행(`EditBuffer.lines` 인덱스)에 기록,
  정렬 permutation은 뷰 표시 순서만 매핑. 편집해도 정렬은 유지된다.
- **저장은 정렬된 순서로**: 정렬이 적용돼 있으면 permutation 순서로 줄을 써서,
  화면에 보이는 순서 그대로 파일에 저장한다(사용자 요청).
- **저장 시 인코딩 변환**: 저장/다른 이름으로 저장 다이얼로그에서 대상 인코딩을
  선택. 줄 문자열(String, 항상 UTF-8 in-memory)을 대상 인코딩으로 재인코딩.

## 편집 데이터 모델

```rust
/// 편집 모드의 인메모리 문서. 파일 전체를 줄 단위로 보관한다.
/// lines[i] = i번째 논리 행의 텍스트(개행 제외). 세퍼레이터 모드에서도
/// 줄 원문을 그대로 들고, 셀 편집 시 필드 분리→교체→재조립으로 갱신한다.
pub struct EditBuffer {
    pub lines: Vec<String>,
    /// 저장되지 않은 변경이 있는지(닫기/열기 시 경고용).
    pub dirty: bool,
    /// 원본 파일의 개행 스타일(저장 시 재현). 감지 실패 시 플랫폼 기본.
    pub newline: Newline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Newline { Lf, CrLf }
```

- `Document`에 `edit: Option<EditBuffer>` 추가. `None`=뷰 전용(mmap), `Some`=편집 중.
- 편집 모드에서 렌더는 `doc.index.line_range` 대신 `edit.lines[logical]`을 읽는다.
  뷰/편집을 가르는 헬퍼(`logical_line(doc, i) -> Option<Cow<str>>`)를 한 곳에 둔다.

## 편집 진입/이탈

- **진입**: 도구 메뉴 "편집 모드" 토글(또는 툴바 버튼). 클릭 시 파일 전체를 현재
  인코딩으로 디코딩해 `EditBuffer.lines`를 채운다. 큰 파일은 진행률을 상태바에
  표시(백그라운드 로드 후 완료 시 편집 활성). 로드 완료 전엔 편집 비활성.
- **이탈**: 편집 모드 끄기. `dirty`면 "저장하지 않은 변경이 있습니다" 경고.
- **로드 방식**: `source.as_bytes()`를 개행(`\n`)으로 분할하되, memchr로 개행 위치를
  찾아 각 구간을 `decode_line`으로 디코딩. `\r\n`이면 `\r` 제거하고 `newline=CrLf`.
  (인덱싱과 별개로 한 번 더 순회하지만, 편집은 사용자가 명시적으로 켜는 것이라 허용.)

## 텍스트 모드 편집

- egui `TextEdit` 멀티라인을 파일 전체에 그대로 쓰는 것은 대용량에서 불가(전체
  문자열 재레이아웃). 따라서 **줄 배열 위에 직접 편집 상태를 얹는다**:
  - 커서 = `(line: usize, col_char: usize)`(문자 단위).
  - 선택 = `(anchor, caret)` 두 커서 위치. 정규화해 시작<끝.
  - 키 입력: 문자 삽입, Backspace/Delete(줄 경계면 병합), Enter(줄 분할),
    방향키/Home/End, Shift+이동=선택 확장, Ctrl+A/C/X/V.
  - **드래그 선택**: 마우스 down=anchor, drag=caret 갱신, up=확정. 줄에 걸친 범위.
  - **통삭제**: 선택 범위 삭제(여러 줄이면 시작 줄 앞부분 + 끝 줄 뒷부분을 병합).
  - **통붙여넣기**: 클립보드 텍스트를 커서 위치에 삽입(개행 포함 시 줄 분할).
- 렌더: 각 줄을 `body.rows`로 그리되, 커서/선택이 있는 줄만 커서 막대·선택 하이라이트
  를 painter로 덧그린다. 편집은 키/마우스 이벤트를 그 줄 rect에서 받아 처리.
- **비목표(YAGNI)**: undo/redo 다단계, IME 조합, 문법 하이라이트, 워드랩. (undo는
  후속. 최소 1단계 undo도 이번 범위 밖 — 필요하면 별도 사이클.)

## 세퍼레이터 모드 편집(셀)

- **셀 편집**: 셀 더블클릭(또는 선택 후 F2/타이핑) → 그 셀만 인라인 `TextEdit`
  단일라인으로 편집. 확정(Enter/포커스 아웃) 시:
  1. `split_fields(lines[logical], delim)`로 필드 벡터를 얻고,
  2. 해당 col 필드를 새 값으로 교체(col이 현재 필드 수보다 크면 빈 필드로 패딩),
  3. 구분자로 다시 join(따옴표 필요 필드 = 값에 delim/따옴표/개행 포함 시 `"`로
     감싸고 내부 `"`는 `""`로 이스케이프)해 `lines[logical]` 갱신.
- **셀 범위 드래그 선택**: `(r0,c0)`~`(r1,c1)` 사각 영역. 마우스 down=시작 셀,
  drag=끝 셀, up=확정. 선택 영역 파란 음영(기존 컬럼 선택 음영 재사용).
- **통삭제(셀)**: 선택 사각 영역의 각 셀 값을 빈 문자열로. (행 자체를 지우는 것은
  "행 삭제" 별도 동작.)
- **통복사(셀)**: 선택 영역을 TSV(행=\n, 열=\t)로 클립보드에 복사.
- **통붙여넣기(셀)**: 클립보드 TSV를 파싱해 선택 시작 셀부터 그리드로 덮어쓴다.
  (붙여넣는 그리드가 파일 경계를 넘으면 행/열을 확장 — 행 추가/필드 패딩.)
- **행 삽입/삭제**: 선택 행 위/아래 빈 행 삽입, 선택 행 삭제. `lines`에 `insert`/
  `remove`. 정렬 permutation이 있으면 인덱스 시프트 필요(아래 "정렬 공존" 참조).

## 우클릭 컨텍스트 메뉴

- egui `response.context_menu(|ui| …)`로 구현. 우클릭 위치가 현재 선택 안이면 선택
  대상으로, 밖이면 그 셀/줄을 먼저 선택한 뒤 메뉴.
- **텍스트 모드 항목**: 잘라내기 / 복사 / 붙여넣기 / 삭제 / 전체 선택.
- **셀 모드 항목**: 잘라내기 / 복사 / 붙여넣기 / 셀 내용 지우기 / ─ / 위에 행 삽입 /
  아래에 행 삽입 / 행 삭제.

## 정렬과 편집 공존

- 편집은 항상 **논리 행 인덱스**(`edit.lines`의 위치)에 대해 일어난다. 렌더에서
  뷰 행 → (permutation 있으면) 논리 행으로 매핑하는 기존 경로를 그대로 쓴다:
  `logical = permutation[view_row]` (없으면 `data_start + view_row`).
- **값 편집(셀/텍스트 내용 변경)**: permutation은 논리 행 번호를 담으므로 값만
  바뀌면 permutation은 유효하다. 재정렬하지 않는 한 순서 유지(사용자가 다시
  정렬하면 새 값 기준으로 재계산).
- **행 삽입/삭제**: `lines`의 길이가 바뀌므로 permutation의 인덱스가 어긋난다.
  정책(단순·정확): **행 삽입/삭제 시 정렬을 해제**(permutation 폐기, 원본 순서
  복귀). 상태바에 "행 편집으로 정렬 해제됨" 안내. (permutation 실시간 리매핑은
  복잡도 대비 이득이 적어 YAGNI. 값 편집은 정렬 유지, 구조 편집만 해제.)
- 편집 모드에서 정렬을 새로 하려면 `EditBuffer.lines`에서 키를 뽑아야 한다. 기존
  `extract_and_sort`는 mmap+offset 기반이므로, 편집 모드용 인메모리 정렬 경로
  (`sort_edit_buffer(lines, specs) -> Vec<u32>`)를 별도로 둔다(같은 키 인코딩 재사용).

## 저장 / 다른 이름으로 저장

- **다른 이름으로 저장 다이얼로그**: rfd `save_file()`로 경로 선택 + 인코딩
  ComboBox(UTF-8/CP949/UTF-16LE/BE) + "BOM 포함" 체크(UTF-8/UTF-16). 확정 시 스트리밍.
- **저장(덮어쓰기)**: 현재 경로가 있으면 그 경로로. 인코딩은 현재 문서 인코딩 기본,
  다이얼로그에서 바꿀 수 있게(간단히는 "저장"은 현재 인코딩, "다른 이름"에서만
  인코딩 선택 — 최소 구현). → **결정: 저장/다른이름 모두 인코딩 선택 가능**하게 통일.
- **쓰기 순서**: 정렬이 적용돼 있으면 permutation 순서로, 아니면 `lines` 순서로.
  헤더가 있으면 헤더(논리 행 0)를 맨 앞에 고정하고 데이터 행을 순서대로.
- **스트리밍**: 임시 파일(`<대상>.tmp`)에 `BufWriter`로 줄마다
  `encode(line + newline, target_enc)`를 써 내려가고, 완료 후 원자적 rename으로
  대상 교체. 실패 시 임시 파일 정리(원본 보존). 큰 파일 대비 진행률 표시.
- **인코딩 재인코딩**: `encoding_rs`의 각 인코더로 `encode`. UTF-16은 2바이트 단위.
  BOM 옵션이 켜지면 파일 맨 앞에 해당 BOM 바이트를 먼저 쓴다.
- **저장 후**: `dirty=false`. 다른 이름으로 저장이면 `path_label`/경로 갱신. 원한다면
  저장한 파일을 다시 뷰 전용으로 열지 여부는 사용자에게 — 최소 구현은 편집 상태 유지.

## 파일 구조

### 신설: src/edit.rs
- `EditBuffer { lines, dirty, newline }`, `Newline`.
- `load_edit_buffer(source, enc) -> EditBuffer`: mmap 바이트를 개행으로 분할·디코딩.
- `TextCursor { line, col }`, `TextSelection { anchor, caret }` + 정규화/삭제/삽입.
- `CellSelection { r0, c0, r1, c1 }`(정규화) + 셀 통복사/삭제/붙여넣기 헬퍼.
- `set_cell(lines, logical, col, value, delim)`: 필드 교체+재조립(따옴표 이스케이프).
- `insert_row`, `remove_row`.
- `sort_edit_buffer(lines, specs, has_header) -> Vec<u32>`: 인메모리 다중 정렬
  (기존 sort.rs 키 인코딩 재사용, offset 대신 lines 직접).

### 신설: src/save.rs
- `SaveOptions { path, enc, bom }`.
- `write_streaming(lines, order: Option<&[u32]>, has_header, opts, progress) -> io::Result<()>`:
  임시 파일에 스트리밍 후 원자적 rename. order=permutation(정렬 순서) 또는 None.
- `encode_line(s, enc) -> Vec<u8>`, BOM 바이트 헬퍼.

### 수정: src/app.rs
- `Document`에 `edit: Option<EditBuffer>`, 편집 UI 상태(text_sel/cell_sel/editing_cell 등).
- 메뉴바 파일 메뉴에 "저장", "다른 이름으로 저장…". 도구/툴바에 "편집 모드" 토글.
- `render_table`/`render_text`가 편집 모드면 `EditBuffer`에서 읽고 편집 이벤트 처리.
- 저장 다이얼로그(`render_save_dialog`), 편집 로드 진행률, dirty 경고.
- 편집 로드/저장은 백그라운드 스레드 + 진행률(기존 SortJob 패턴 재사용).

### 수정: src/parse.rs
- 저장용 필드 조립 시 따옴표 이스케이프 규칙을 함수로: `join_fields(&[String], delim)`.

## 엣지 케이스

- 빈 파일 편집 → `lines = [""]`(빈 한 줄)로 시작.
- 편집 모드에서 구분자/인코딩 변경 → 셀 분리 기준이 바뀜. 값은 그대로 두되 표시만
  재분리. (편집 중 인코딩 변경은 in-memory String엔 영향 없음 — 저장 인코딩만 별개.)
- UTF-16 원본 편집 로드: `decode_line`이 UTF-16 처리하므로 lines는 UTF-8 String.
  저장 시 대상 인코딩으로 재인코딩.
- 매우 긴 줄(개행 없는 거대 파일): `lines`가 한 개 초거대 String — 텍스트 편집은
  느릴 수 있으나 동작. (표 모드는 애초에 개행이 있어야 행 개념.)
- 저장 중 디스크 부족/권한 오류 → 임시 파일 삭제, 원본 보존, 에러 표시.
- 정렬 순서 저장 시 헤더 중복 방지(헤더는 permutation에 포함 안 됨 — data_start=1).

## 테스트 전략

edit.rs / save.rs 순수 함수 단위 테스트(GUI 없이):
1. `load_edit_buffer`: LF/CRLF 분할, 마지막 개행 유무, 빈 파일 → `[""]`.
2. 텍스트 편집: 삽입/삭제/줄 분할(Enter)/줄 병합(경계 Backspace), 범위 삭제(멀티라인).
3. `set_cell`: 필드 교체, col 패딩, delim/따옴표 포함 값 이스케이프.
4. 셀 통삭제/붙여넣기: TSV 파싱→그리드 덮어쓰기, 경계 확장.
5. `insert_row`/`remove_row`.
6. `sort_edit_buffer`: 기존 정렬과 동일 결과(회귀), 값 편집 후 재정렬.
7. `write_streaming`: 각 인코딩(UTF-8/CP949/UTF-16LE/BE) 왕복(write→read back), BOM,
   정렬 순서(order 지정) 반영, CRLF/LF 재현, 원자적 rename(임시→대상).

## 비목표 (YAGNI)

- undo/redo(후속 사이클).
- piece table / mmap 조각 편집(전부 RAM 로드로 충분).
- 문법 하이라이트, 워드랩, IME 조합, 찾기/바꾸기(별도).
- 편집 중 실시간 permutation 리매핑(구조 편집 시 정렬 해제로 단순화).
- 부분 로드/스트리밍 편집(초대용량에서 RAM 초과 시) — 필요해지면 별도 설계.
