# 찾기 하이라이트 · 스크롤 마커 · 별도 창 · Whole cell 옵션 — 설계

작성일: 2026-07-24
브랜치: `feat/editing-and-save`

## 목표

찾기를 EMEditor처럼 만든다. 네 가지를 한 묶음으로:

1. **전체 매치 하이라이트** — 검색어가 있는 모든 곳을 옅은 보라 음영으로. 화면에
   보이는 행뿐 아니라, 스크롤해서 나타나는 행도.
2. **스크롤 마커** — 문서 전체에서 매치가 있는 위치를, 스크롤바 옆 세로 거터에
   보라 눈금으로. 전체 분포가 한눈에 보인다.
3. **current match 강조** — Find Next/Prev로 점프하는 현재 매치는 더 **진한** 보라.
4. **Whole cell 옵션** — 표 모드에서 셀 전체가 정확히 일치할 때만 매치. 부분 매칭은
   건너뛴다.

그리고 찾기/바꾸기 UI를 **하단 바에서 별도 창(egui::Window)으로** 옮긴다 —
하단 바가 좁아 불편하다.

## 측정으로 확정된 사실

`src/find.rs` 실코드로 2GB / 2200만 행 CSV를 벤치마크했다(scratchpad/scanbench):

| 방식 | 희귀 검색어 | 흔한 검색어(전 행 매치) |
|---|---|---|
| 현재 `find_in_line` 행 루프 | 25.7초 | 24.7초 |
| **파일 전체 `memchr::memmem` + offset 이진탐색** | **0.05초** | **0.85초** |

전체 스캔은 **파일을 바이트 단계에서 `memmem`으로 훑고, 히트 offset을 줄 인덱스로
이진탐색 매핑**하면 실용적으로 빠르다. 행마다 `String` 디코딩 + `Vec<char>` 할당을
하는 현재 경로가 느림의 원인이다.

**결정**: 매치 상한 없이 전부 저장한다(사용자 지시). 단 스크롤 마커에 필요한 것은
**행 번호뿐**이므로 `Vec<u32>`로 저장한다(2200만 행 = 88MB). `Match` 구조체 전체를
전부 저장하지 않는다.

## 사용자가 내린 결정

- 스캔 상한 **없음** — 전부 스캔, 전부 저장.
- 매치 옵션은 **3지 라디오**: Partial / Whole word / Whole cell (서로 배타적).
  Whole cell은 표 모드에서만 활성.
- 찾기/바꾸기는 **별도 창**.
- 하이라이트: 전체 매치 = 옅은 보라, current = 진한 보라, 스크롤 마커.

## 아키텍처 개요

### 데이터 흐름

```
검색어/옵션 변경  ─►  scan_all_matches(doc)  ─►  doc.match_rows: Vec<u32>   (스크롤 마커용)
                                             └►  doc.match_generation += 1  (캐시 무효화)

렌더(매 프레임)  ─►  화면에 보이는 각 행에 대해서만 find_in_line 재실행
                     (전체 매치 음영 — 값싸다, 보이는 행만)
                 ─►  스크롤 거터에 doc.match_rows를 눈금으로
                 ─►  current match(doc.last_match)는 진한 음영
```

**핵심 설계 판단**: 전체 매치 하이라이트는 **화면에 보이는 행만** 매 프레임
`find_in_line`으로 다시 계산한다(가상 스크롤이라 보이는 행은 수십 개뿐, 값싸다).
스크롤 마커만 문서 전체 스캔 결과(`match_rows`)를 쓴다. 이렇게 나누면:

- 전체 매치 위치(글자 단위 col/len)를 200만 개씩 저장할 필요가 없다 — 보이는 행만
  즉석 계산.
- 스크롤 마커는 행 번호만 있으면 되므로 `Vec<u32>` 하나로 충분.

### 언제 전체 스캔을 도는가

`scan_all_matches`는 **비싸므로(최악 0.85초/2GB) 매 프레임 돌리면 안 된다.**
다음일 때만:

- 검색어(`find_query`)가 바뀌었을 때
- 옵션(`find_opts`)이 바뀌었을 때
- 문서 내용이 바뀌었을 때(편집/정렬/Replace) — `doc`에 이미 있는 dirty/버전
  신호를 재사용하거나, 편집 계열 동작 뒤 `match_rows`를 무효화한다

무효화는 `doc.match_query: Option<(String, FindOptions)>`를 저장해 두고, 현재
`find_query`/`find_opts`와 다르면 재스캔하는 방식으로 단순하게 한다. 편집으로
버퍼가 바뀌면 `match_query = None`으로 지워 다음 프레임에 재스캔되게 한다.

**뷰 모드 부분 인덱싱 주의**: 뷰 모드에서 인덱서가 아직 문서 끝까지 도달하지
않았으면, 스캔은 인덱싱된 행까지만 본다(기존 찾기와 같은 한계). 인덱싱이
진행 중이면 `match_rows`도 인덱싱이 끝난 뒤 재스캔이 필요하다 — 인덱서 완료
신호가 이미 있으면 그때 `match_query = None`으로 무효화한다. 없으면 이 한계를
그대로 두고 리포트에 남긴다.

## 상세 설계

### S-1. `src/find.rs` — 매치 옵션 재구성

지금 `FindOptions { match_case, whole_word }`는 `whole_word`가 bool이다. Whole cell을
추가하면서 **배타적 3지**로 바꾼다.

```rust
/// 매치 범위. 서로 배타적(UI에서 라디오).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchScope {
    /// 부분 일치(기본). 행 어디든 needle이 나오면 매치.
    Partial,
    /// 단어 단위. 매치 앞뒤가 단어 문자가 아닐 때만.
    WholeWord,
    /// 셀 전체 일치. 셀(필드) 전체가 needle과 정확히 같을 때만.
    /// 표 모드에서만 의미가 있다. 텍스트 모드에서는 Partial처럼 동작한다
    /// (셀 개념이 없으므로) — 또는 "행 전체 일치"로 해석한다. 아래 S-2 참조.
    WholeCell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub scope: MatchScope,
}

impl Default for FindOptions {
    fn default() -> Self {
        FindOptions { match_case: false, scope: MatchScope::Partial }
    }
}
```

기존 `whole_word: bool`을 쓰던 모든 곳(테스트 포함)을 `scope`로 바꾼다.
`whole_word: true` → `scope: MatchScope::WholeWord`, `false` → `Partial`.
기존 테스트의 **의미는 유지**한다.

### S-2. `find_in_line`에서 Whole cell 처리

`find_in_line`은 delimiter를 모른다(행 문자열만 받는다). Whole cell 판정은 셀
경계를 알아야 하므로, **delimiter를 인자로 받는 새 함수**로 처리한다.

```rust
/// 한 행에서 needle이 나오는 (col, len)들. Whole cell 모드면 delim으로 셀을
/// 나눠, 셀 전체가 needle과 정확히 같은 셀의 (셀 시작 col, 셀 길이)만 반환한다.
///
/// delim이 None이면(텍스트 모드) Whole cell은 "행 전체 일치"로 해석한다 —
/// 행 전체가 needle과 같을 때만 (0, 행 길이) 하나를 반환.
pub fn find_in_line_scoped(
    hay: &str,
    needle: &str,
    opts: &FindOptions,
    delim: Option<u8>,
) -> Vec<(usize, usize)>;
```

- `scope != WholeCell`이면 기존 `find_in_line`을 그대로 위임 호출한다(동작 불변).
- `scope == WholeCell` + `delim == Some(d)`이면:
  - 행을 `d`로 셀 분할한다. **기존 `parse::field_slice`를 재사용**해 셀 경계를
    얻는다(따옴표 처리가 이미 올바르다). 셀 내용을 needle과 비교:
    - `match_case`면 그대로, 아니면 양쪽 `to_lowercase()` 비교.
    - **비교는 셀 전체 == needle 전체**. 부분 매칭은 매치 아님.
  - 매치된 셀의 (그 셀의 행 내 char 시작 인덱스, 셀 char 길이)를 반환.
    **char 인덱스로 변환**해야 한다(하이라이트가 char 단위). field_slice는 바이트
    오프셋을 주므로 char로 환산.
- `scope == WholeCell` + `delim == None`(텍스트 모드)이면 행 전체 == needle일 때만
  `(0, char_len)` 하나.

`find_next`/`find_prev`/`matching_lines`/`replace_all`/`replace_in_line`도 Whole cell을
존중해야 한다. 이들은 현재 `find_in_line`을 부르므로, **delim을 받도록 시그니처를
확장**하거나, `get_line`처럼 "이 행에서 매치를 찾는" 클로저를 주입받게 한다.
**판단**: `find.rs`가 delimiter를 흘려받는 것이 가장 단순하다. `find_next` 등에
`delim: Option<u8>`를 추가하고 내부에서 `find_in_line_scoped`를 쓰게 한다.
호출부(app.rs)는 표 모드면 `Some(doc의 delim)`, 텍스트 모드면 `None`을 넘긴다.

**바꾸기와 Whole cell의 상호작용**: Whole cell 모드에서 Replace는 **셀 전체를**
replacement로 바꾼다(부분이 아니라). replacement에 delimiter가 들어 있으면?
→ `sanitize`로 개행만 막고 delimiter는 그대로 둔다(사용자가 일부러 넣었을 수 있다).
이건 드문 경우이므로 단순하게 처리하고 리포트에 남긴다.

### S-3. `src/find.rs` — 전체 행 스캔(스크롤 마커용)

```rust
/// needle이 있는 논리 행 번호를 전부 모은다. 스크롤 마커용이므로 행 번호만.
/// get_line이 None인 행은 건너뛴다(부분 인덱싱).
///
/// 이건 `matching_lines`와 거의 같지만, 반환이 `Vec<u32>`이고 호출 규모가
/// 크다(문서 전체). app.rs 쪽에서 바이트 prefilter를 적용할 수 있게, 이
/// 함수는 순수 판정만 하고 실제 대량 스캔의 최적화는 app.rs가 담당한다.
```

**성능 결정**: `find.rs`는 `Document`/`Source`/mmap을 모르므로, 벤치마크가 증명한
"파일 전체 memmem" 최적화를 `find.rs` 안에서 할 수 없다. 두 갈래:

- **(가)** `find.rs::matching_lines`(이미 있음, `get_line` 클로저)를 그대로 쓰되,
  app.rs가 `get_line`을 넘길 때 **행 바이트를 먼저 memmem으로 걸러** 매치 가능성이
  없는 행은 `find_in_line`을 아예 안 돌리게 한다. 하지만 클로저는 이미 `String`을
  받으므로 바이트 prefilter를 넣을 자리가 없다.
- **(나)** 스크롤 마커 스캔을 **app.rs에 두고**, mmap 바이트에 직접 `memmem`을
  돌려 히트 offset을 `LineIndex`로 이진탐색 매핑한다(벤치의 C 방식). 편집 모드면
  `EditBuffer.lines`를 순회하며 각 행 바이트에 memmem. Whole word/cell 정밀 판정은
  memmem이 건 행에만 `find_in_line_scoped`로 확인.

**(나)를 택한다.** 벤치가 (나)만 실용적임을 보였다(160배). `find.rs`에는 순수
판정 함수(`find_in_line_scoped`)만 두고, 대량 스캔 오케스트레이션은 app.rs의
`scan_all_matches(doc) -> Vec<u32>`가 한다. 이 함수는:

1. `needle` 바이트로 `memchr::memmem::Finder`를 만든다.
2. **뷰 모드**: `source.bytes()` 전체에 `finder.find_iter`, 각 히트 offset을
   `index.snapshot()`의 offset 배열에 이진탐색해 행 번호를 얻는다. 같은 행 중복
   제거(연속 히트가 같은 행이면 한 번만). 그 행들에 대해서만
   `find_in_line_scoped`로 case/word/cell 정밀 판정(memmem은 대소문자 구분이고
   부분 일치이므로, ignore_case나 whole_word/cell이면 후처리 필요).
   - **ignore_case 주의**: memmem은 대소문자를 구분한다. ignore_case면 바이트
     prefilter를 needle 소문자로 못 한다. 이 경우 prefilter를 건너뛰고 행 단위
     `find_in_line_scoped`로 폴백하거나(느림), needle의 대소문자 변형들로
     여러 번 memmem을 돌린다(ASCII면 간단, 유니코드면 복잡).
     **판단**: ignore_case일 때는 **ASCII 케이스만 prefilter 최적화**(needle이
     ASCII면 대/소문자 두 벌 memmem), 비ASCII가 섞이면 행 단위 폴백. 결정과
     이유를 주석·리포트에 남긴다. 정확성은 어느 경로든 `find_in_line_scoped`가
     최종 판정하므로 보장된다.
3. **편집 모드**: `EditBuffer.lines`를 순회, 각 행에 memmem prefilter 후
   `find_in_line_scoped` 확인.

memmem이 못 거르는 경우(ignore_case 폴백)라도 **정확성은 항상 유지**되어야 한다 —
느릴 뿐. 이걸 테스트로 증명한다.

### S-4. `Document` 상태 추가

```rust
/// 스크롤 마커용: 검색어가 있는 논리 행 번호(전체 문서). 마커만 이걸 쓴다.
pub match_rows: Vec<u32>,
/// match_rows가 어떤 (검색어, 옵션)으로 만들어졌는지. 현재 find_query/find_opts와
/// 다르면 재스캔. None이면 무효(다음 프레임 재스캔).
pub match_query: Option<(String, crate::find::FindOptions)>,
```

`open_path`와 `add_document`(추출) 두 생성 지점에서 초기화(`Vec::new()`, `None`).

편집/정렬/Replace/undo 등 **버퍼를 바꾸는 모든 경로 뒤에 `match_query = None`**.
지점이 많으므로, 이미 `dirty = true`를 세우는 자리들을 찾아 같이 무효화한다.
누락되면 마커가 낡은 위치를 가리키므로, **버퍼를 바꾸는 지점에서 dirty와
match 무효화를 한 헬퍼로 묶는 것**을 고려한다(판단).

### S-5. 색상 (theme.rs)

```rust
/// 전체 매치 음영(옅은 보라). 데이터가 읽히도록 알파를 낮게.
pub fn find_match_bg() -> Color32 { Color32::from_rgba_unmultiplied(150, 90, 200, 55) }
/// 현재 매치 음영(진한 보라). find_match_bg보다 확실히 진하게.
pub fn find_current_bg() -> Color32 { Color32::from_rgba_unmultiplied(140, 60, 190, 130) }
/// 스크롤 마커 눈금(보라, 불투명에 가깝게 — 얇으므로 잘 보여야).
pub fn find_marker() -> Color32 { Color32::from_rgba_unmultiplied(150, 70, 200, 200) }
```

값은 예시다. 밝은 테마(순백 데이터 배경)에서 실제로 잘 보이고 글자를 안 가리는지
구현자가 확인해 조정한다. current가 전체보다 확실히 진해야 한다는 관계만 지킨다.

### S-6. 렌더 — 전체 매치 하이라이트(보이는 행)

**텍스트 모드(`render_text`)**: 편집 모드 경로(app.rs:3988~)는 이미 galley를
직접 레이아웃하고 `x_of(char)`로 음영 rect를 그린다(선택 음영이 그 예). 매치
음영을 **선택 음영과 글자 사이**에 추가한다:

- 그 행에서 `find_in_line_scoped(line, query, opts, None)`을 호출(query 비면 스킵).
- 각 (col, len)에 대해 `x_of(col)..x_of(col+len)` rect를 `find_match_bg()`로.
- 그 매치가 `doc.last_match`와 같으면(line·col·len 일치) `find_current_bg()`로.

**뷰 전용 모드**(app.rs:3984, 지금은 `Label`): 하이라이트하려면 galley 경로가
필요하다. 뷰 모드도 편집 모드처럼 galley를 레이아웃해 매치 음영 + 글자를 직접
그리도록 바꾼다. **단 캐럿/선택/드래그 상호작용은 뷰 모드에 없으므로** 음영과
글자만. `find_query`가 비어 있으면 기존 `Label` 경로를 그대로 써서 회귀를 피한다
(검색 중이 아닐 때는 지금과 똑같이 동작).

**표 모드(`render_table`)**: 각 데이터 셀을 그릴 때, 그 셀 텍스트에서 매치를 찾아
음영. 표는 셀 단위로 그리므로, 셀 텍스트에 대해 `find_in_line_scoped`를 부르되
delim은 이미 셀로 쪼갠 뒤이므로 `None`(Partial/WholeWord는 셀 안에서, WholeCell은
셀==query). 표 셀도 지금 `Label`로 그리면 galley 경로로 바꿔야 부분 음영이 된다.
표는 셀이 많으므로 **query가 있을 때만** galley 경로, 없으면 기존 Label 경로.

### S-7. 렌더 — 스크롤 마커 거터

`Table::body`는 `ScrollAreaOutput`을 삼켜(`()` 반환) 스크롤 트랙 rect를 노출하지
않는다. 그래서 **egui 기본 스크롤바 위에 겹쳐 그릴 수 없다.** 대신 테이블/텍스트
영역 **오른쪽에 얇은 세로 거터(marker gutter)를 직접 만든다** — EMEditor의 우측
마커 바와 같은 방식.

- CentralPanel 안에서, 데이터 영역을 그리기 전에 오른쪽에서 폭 ~14px를
  `ui.allocate_ui`/`SidePanel`류로 떼어 거터로 쓴다. 판단해서 배치.
- 거터 세로 길이를 전체 논리 행 수에 매핑한다: 행 `r`의 마커 y =
  `gutter_top + (r / line_count) * gutter_height`.
- `doc.match_rows`의 각 행에 대해 그 y에 `find_marker()` 색으로 2px 높이 눈금을
  그린다. 행이 아주 많으면 여러 행이 같은 픽셀에 겹치는데, 그냥 겹쳐 그리면
  된다(누적 알파로 밀집 구간이 진해져 오히려 분포가 잘 보인다).
- **거터 클릭 → 그 위치로 점프**: 거터를 클릭하면 y를 행 번호로 역산해
  `pending_scroll_row`로 스크롤한다(값싸고 유용하다). 선택.
- current match(`doc.last_match`)의 행은 거터에도 `find_current_bg()` 진한 눈금으로.

거터는 `match_rows`가 비어 있으면(검색 안 함) **그리지 않는다** — 데이터 폭을
아끼기 위해. 마커가 있을 때만 나타난다.

### S-8. 찾기/바꾸기를 별도 창으로

지금 하단 `TopBottomPanel::bottom("find")`를 **`egui::Window`로** 바꾼다.

- `egui::Window::new("Find & Replace")`, `.open(&mut show_find)`,
  `.collapsible(false)`, `.resizable(false)`. 위치는 기본(우상단 근처) 또는
  `.default_pos`로 적당히.
- 내용은 세로로 여유 있게 배치(창이라 넓다):
  - `Find:` + 입력란(폭 넉넉히, 260+)
  - `Replace:` + 입력란
  - **매치 옵션 라디오 3지**: `ui.radio_value(&mut scope, MatchScope::Partial, "Partial")`
    등. `Whole cell`은 표 모드가 아니면 `add_enabled_ui(is_table, ...)`로 비활성.
  - `Match case` 체크박스(라디오와 독립).
  - 버튼 줄: `Find Prev` `Find Next` `Replace` `Replace All` `Extract Rows`.
    바꾸기 계열은 편집 모드에서만 활성(기존과 동일).
  - `find_status` 문구 + 매치 개수(예: `"12 matches"` — `match_rows.len()`).
- 라벨은 `crate::theme::chrome_text`.
- **단축키 유지**: `Ctrl+F`가 창을 열고 입력란에 포커스, `Enter`=Find Next,
  `F3`=Find Next, `Escape`=창 닫기. 기존 게이트(다른 위젯 포커스 시 양보,
  `find_keys_live`)를 그대로 따른다. 창이 열려도 데이터 영역 단축키와 충돌 안 나게.
- **`tab_bar_locked()`와의 관계**: 이 창은 저장/확인 다이얼로그가 아니므로
  `tab_bar_locked`에 넣지 않는다(찾기 창이 떠 있어도 탭 전환은 돼야 한다).
  탭을 전환하면 새 활성 문서의 `show_find`/`find_query`를 보게 되는데, 찾기
  상태는 문서별이므로 자연스럽다. 다만 **찾기 창의 입력란이 편집하는 대상이
  활성 문서**임을 확인한다(기존 `apply_find_action`이 활성 doc을 쓰므로 이미 OK).

`render_find_panel`의 반환 인텐트 패턴(클로저 밖에서 적용)을 그대로 유지한다.

## 하지 말 것 (YAGNI)

- 정규식, 다중 파일 grep
- 매치 개수 상한 UI(사용자가 "전부"로 결정)
- 스크롤 마커에 매치 밀도 히트맵 색 그라디언트(단색 누적 알파로 충분)
- 찾기 기록 드롭다운
- 별도 창의 도킹/위치 저장

## 테스트

### `src/find.rs`

- [ ] `match_scope_default_is_partial`
- [ ] `whole_cell_matches_exact_cell_only` — `"a,bb,ccc"`에서 delim=`,`, needle=`bb`,
      WholeCell → 셀 1(col 2, len 2)만. needle=`b` → 매치 없음(부분 아님).
- [ ] `whole_cell_char_index_with_hangul` — `"가,나다,x"`에서 needle=`나다`,
      WholeCell → col이 char 인덱스(2), 바이트 아님.
- [ ] `whole_cell_ignore_case` — `"A,BB,c"`에서 needle=`bb`, WholeCell+ignore_case → 매치.
- [ ] `whole_cell_text_mode_matches_whole_line` — delim=None, 행 전체==needle일 때만.
- [ ] `whole_cell_respects_quotes` — 따옴표 안 delimiter가 셀을 안 쪼갠다
      (field_slice 재사용이 실제로 동작하는지).
- [ ] `scoped_partial_delegates_to_find_in_line` — Partial/WholeWord는 기존과 동일.
- [ ] `find_next_respects_whole_cell` — delim 인자를 넘겼을 때 Whole cell로 이동.
- [ ] `replace_all_whole_cell_replaces_entire_cell`
- [ ] 기존 `whole_word` 테스트들을 `MatchScope::WholeWord`로 이관(의미 유지).

### `src/app.rs`

- [ ] `scan_all_matches_view_mode_finds_all_rows` — 인메모리 Source(추출 경로
      `Source::from_bytes` 재사용)로 문서를 만들고, memmem 스캔이 정답
      (`matching_lines` 브루트포스)과 **같은 행 집합**을 주는지. 여러 인코딩
      (UTF-8, CP949) + ignore_case 폴백 경로 포함.
- [ ] `scan_all_matches_edit_mode` — 편집 버퍼 순회 경로.
- [ ] `scan_ignore_case_matches_brute_force` — ignore_case에서 memmem prefilter가
      뭘 거르든 최종 결과가 브루트포스와 같은지(정확성 불변).
- [ ] `scan_whole_cell_only_full_cells` — Whole cell 스캔이 부분 매치 행을 뺀다.
- [ ] `match_query_invalidates_on_option_change` — query/opts가 바뀌면 재스캔되고,
      안 바뀌면 캐시된 `match_rows`를 그대로 쓴다(재스캔 안 함).
- [ ] `edit_invalidates_match_rows` — 버퍼를 바꾸면 `match_query`가 None이 된다.
- [ ] `whole_cell_option_disabled_in_text_mode` — 순수 함수로 활성 조건을 뽑아
      (예: `whole_cell_enabled(sep) -> bool`) 텍스트 모드면 false. **가드를 인라인
      복붙하지 말 것** — 진짜 조건을 뒤집으면 테스트가 깨져야 한다.
- [ ] `marker_y_maps_row_to_gutter` — 행→거터 y 매핑 순수 함수. 0행은 top,
      마지막 행은 bottom 근처, 클릭 y→행 역산이 왕복.
- [ ] `find_dialog_open_does_not_lock_tab_bar` — 찾기 창이 떠 있어도
      `tab_bar_locked()`이 true가 되지 않는다.

## 검증

```
cargo test
cargo clippy --all-targets
```

- 기존 348 테스트 전부 통과(옵션 이관으로 시그니처가 바뀐 것들은 수정).
- 새 clippy 경고 0.
- `cargo build --release`는 **실행하지 말 것**(앱 실행 중이면 exe 잠김, os error 5).

## 커밋

로직/렌더/창을 나눠 커밋해도 좋다.
```
feat: 찾기 매치 옵션을 3지(Partial/WholeWord/WholeCell)로, Whole cell 순수 로직
feat: 찾기 전체 하이라이트 + 스크롤 마커 거터
feat: 찾기/바꾸기를 별도 창으로
```
