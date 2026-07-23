# 컬럼 선택 + 정렬 설계

> 헤더 클릭으로 컬럼을 선택하고, 그 컬럼 값 기준(문자/숫자 × 오름/내림)으로
> 전체 파일을 정렬한다. 행 바이트는 옮기지 않고 정렬 순서(permutation)만 만든다.

**작성일:** 2026-07-23
**목표:** 대용량 파일도 컬럼 값 기준으로 정렬해 그 순서로 표시. mmap 읽기 전용
모델 유지, 행 재작성 없음.

---

## 핵심 접근: indirect/pointer sort (permutation)

행 바이트를 절대 옮기지 않는다. **`(정렬 키, 원본 행번호)` 쌍만 뽑아 정렬**하고,
결과로 **행번호 permutation**(`Vec<u32>`)을 만든다. 뷰어는
`보이는 행 → permutation[i] → 원본 행번호 → 기존 line_range()`로 렌더한다.
기존 렌더 경로에 인덱싱 한 겹만 얹는 국소 변경이다. (조사 근거:
.readme 및 EMEditor 15.2GB/1억행 7.3초 정렬이 이 방식이라는 방증.)

**범위 = 옵션 A (in-memory par_sort).** `(key, idx)` 배열을 rayon으로 메모리 내
정렬. EMEditor도 정렬 시 행 수 비례 메모리를 그냥 쓰므로(사용자 관찰: 정렬 중
+8GB) RAM 절약에 과하게 매달리지 않는다. 우리는 mmap이라 파일 자체는 RAM에
안 올려 더 유리. `(key,idx)` 배열이 가용 RAM 초과 예상 시에만 거부+안내.
external sort(옵션 B)는 향후 필요 시.

---

## 데이터 흐름

### 1. 컬럼 선택
- 표 모드에서 헤더 셀 클릭 → 그 컬럼 선택. `App.selected_col: Option<usize>`.
- 선택된 컬럼은 헤더+본문 배경색 강조.
- 텍스트 모드(SeparatorMode::None)에서는 컬럼 개념이 없어 선택/정렬 비활성.

### 2. 정렬 트리거
- 툴바에 정렬 컨트롤: **문자↑ / 문자↓ / 숫자↑ / 숫자↓** + **정렬 해제**.
- 컬럼이 선택돼 있고 **인덱싱 완료(Phase::Complete)** 일 때만 활성.
  (전체 행이 있어야 정확한 정렬. 인덱싱 중이면 비활성 + 안내.)

### 3. 키 추출 (병렬 스캔)
- 기존 rayon 병렬 패턴 재사용. 전체 행을 워커로 나눠, 각 행의 `line_range`에서
  선택 컬럼 필드만 파싱.
- **숫자 모드**: 필드를 `f64`로 파싱. 파싱 실패(비수치/빈값)는 정렬에서 **맨 뒤로**.
- **문자 모드**: 필드 원본 문자열. 키가 길 수 있으므로 정렬 키에 **앞 N바이트
  prefix를 인라인** + prefix가 같으면 원본 필드를 다시 읽어 tie-break.
  (1차 구현은 단순화를 위해 전체 문자열 키도 허용 — 아래 "키 표현" 참조.)

### 4. 정렬
- `Vec<(Key, u32)>` 를 `rayon::par_sort_unstable_by_key`로 정렬.
- **안정성**: 키가 같으면 원본 행번호(u32)로 tie-break → 안정 정렬 효과(원본 순서 유지).
- 내림차순은 비교 반전 또는 정렬 후 `reverse`.
- 결과 = `permutation: Vec<u32>` (정렬 순서 → 원본 행번호).

### 5. 렌더링
- `App`(또는 Document)에 `sort: Option<SortState>` 보관. `SortState`는 permutation +
  정렬된 컬럼/방향/종류.
- 표 본문 렌더에서 정렬 상태면 `logical = permutation[row.index()]`, 없으면 기존
  `logical = row.index() + data_start`.
- **헤더 처리 주의**: has_header면 원본 0행(헤더)은 permutation에서 제외해야 한다.
  키 추출/정렬 대상은 데이터 행(`data_start..total`)만. permutation은 데이터 행
  번호들의 재배열.
- 정렬된 컬럼 헤더에 ↑/↓ 화살표 표시.

---

## 키 표현 (SortKey)

```rust
enum SortKey {
    // 숫자 정렬: f64. 비수치는 None → 정렬 시 맨 뒤로.
    Num(Option<OrderedF64>),   // f64는 Ord가 없어 total_cmp 래퍼 사용
    // 문자 정렬: 원본 필드 문자열(1차 구현). 대용량에서 메모리가 크면
    // prefix 인라인으로 최적화(향후).
    Text(String),
}
```

- 숫자: `f64::total_cmp`로 NaN 안전 비교. 비수치는 `None`으로 두고 항상 뒤로.
- 문자: 바이트 사전순(`str`/`Vec<u8>` 기본 Ord). 대소문자 구분은 1차 구현에서
  구분함(단순). 향후 case-insensitive 옵션 여지.
- **메모리**: `(SortKey, u32)`. 숫자면 쌍당 ~16B. 문자면 String 길이만큼. 데이터
  행 수 × 쌍 크기가 가용 RAM 초과 예상 시 정렬 거부 + 안내.

---

## 파일 구조

### 신설: `src/sort.rs`
- `SortKind { Text, Number }`, `SortDir { Asc, Desc }`.
- `SortKey` enum + Ord 구현(숫자 total_cmp, 비수치 뒤로).
- `extract_and_sort(source, index, enc, delim, col, data_start, kind, dir) -> Vec<u32>`:
  병렬 키 추출 + par_sort → permutation. 순수 로직(GUI 무관)이라 단위 테스트 가능.
- 메모리 예산 체크 헬퍼(행 수 × 예상 쌍 크기 vs 가용 RAM 추정치).

### 수정: `src/app.rs`
- `App.selected_col: Option<usize>`, `App.sort: Option<SortState>`.
- `SortState { permutation: Vec<u32>, col: usize, kind: SortKind, dir: SortDir }`.
- 헤더 클릭 → selected_col 설정 + 강조.
- 툴바 정렬 컨트롤(Complete일 때만 활성) → sort.rs 호출 → SortState 저장.
- render_table 본문: sort 있으면 permutation 경유.
- "정렬 해제" → sort = None.

### 수정: `src/index.rs`
- 변경 없음(기존 offset 배열 그대로). permutation은 app 레벨에 둔다.

---

## 엣지 케이스 / 정책

- **정렬은 Phase::Complete에서만.** 인덱싱 중/Paused면 정렬 버튼 비활성 + 툴팁/상태.
- **비수치 값(숫자 정렬)**: 맨 뒤로(오름/내림 무관하게 항상 뒤).
- **텍스트 모드**: 컬럼 없음 → 선택/정렬 UI 비활성.
- **필드 수 부족한 행**: 선택 컬럼이 없는 행 → 빈 문자열/None 키로 취급(뒤로).
- **정렬 후 인코딩/구분자 변경**: 정렬 무효화(sort = None) — 파싱 기준이 바뀌므로.
- **RAM 초과**: 데이터 행 수 × 쌍 크기 추정이 임계 초과면 거부 + "메모리 부족으로
  정렬 불가(외부정렬 미지원)" 안내.
- **has_header**: 헤더 행은 정렬에서 제외, 항상 맨 위 고정.

---

## 테스트 전략

sort.rs를 GUI 없이 순수 함수로 단위 테스트:

1. **문자 오름/내림**: 작은 CSV에서 선택 컬럼 문자 정렬 → permutation이 기대 순서.
2. **숫자 오름/내림**: "10 < 9" 함정(문자면 "10"<"9", 숫자면 9<10) — 숫자 정렬이
   수치 순서를 내는지.
3. **비수치 혼재(숫자 정렬)**: 숫자+비수치 → 비수치가 맨 뒤.
4. **안정성**: 같은 키의 행들이 원본 순서 유지.
5. **필드 부족 행**: 컬럼 없는 행이 뒤로.
6. **permutation 렌더 동치**: permutation 적용 결과 행 순서가 정렬 기준과 일치.

app.rs: 헤더 클릭 선택 상태, 정렬 후 렌더가 permutation 경유하는지(헬퍼로 검증).

---

## 비목표 (YAGNI)

- external merge sort(옵션 B) — RAM 초과 초대용량. 향후.
- 다중 컬럼 정렬 — 단일 컬럼만.
- case-insensitive / locale-aware 문자 정렬 — 바이트 사전순.
- permutation 디스크 mmap — 1차는 메모리. 초대용량에서 필요 시.
- 정렬 결과 캐시/토글 여러 개.
