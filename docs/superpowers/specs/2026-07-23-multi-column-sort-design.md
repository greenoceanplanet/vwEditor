# 다중 컬럼 정렬 설계

> 여러 컬럼을 1차/2차/... 우선순위로 정렬. 툴바에서 "다중 컬럼 정렬"을 열면
> 모달 창에서 기준(컬럼·문자/숫자·오름/내림) 목록을 구성해 정렬한다.

**작성일:** 2026-07-23
**목표:** 단일 컬럼 정렬을 다중 기준으로 확장. 기존 고속 정렬 인프라(락 없는
snapshot, 순차 순회, 정수 키, 백그라운드+진행률)를 그대로 재사용.

---

## 결정 사항 (확정)

- **고정 개수 키 배열**: 최대 4개 정렬 기준. 키 = `([u64; MAX_KEYS], u32 row)`.
- **8바이트 prefix 비교(단순화)**: 다중 컬럼은 tie-break 원본 재접근 없이 각 기준을
  앞 8바이트 u64 키로만 비교. 8B 넘어 같으면 다음 정렬 기준으로 넘어감(그것도
  같으면 최종적으로 원본 행번호로 안정화).
- **방향을 키에 인코딩**: 내림차순 기준은 키를 반전(`!key`)해 배열에 담는다.
  이러면 오름/내림이 섞여도 `[u64; N]` 배열의 단일 정수 비교로 전체 순서가
  나온다(비교 클로저에 방향 분기 불필요).
- **UI**: 모달 창, 기준 목록 + 행 추가/삭제. 맨 위가 1차 정렬.

## 키 구조

```rust
const MAX_KEYS: usize = 4;

/// 하나의 정렬 기준.
struct SortSpec {
    col: usize,
    kind: SortKind,   // Text | Number
    dir: SortDir,     // Asc | Desc
}

/// 다중 키 정렬 레코드. keys를 앞에서부터 정수 비교(Ord 자동)하고,
/// 전부 같으면 row로 안정화. keys는 미사용 슬롯을 0으로 패딩.
/// (u64;4 = 32B + u32 = 36B, 정렬 패딩으로 40B/행. 3억행 ≈ 12GB — 단일보다
///  크지만 다중 정렬은 드물고, 캐시 친화적 정수 비교라 정렬 자체는 빠르다.)
type MultiKey = [u64; MAX_KEYS];
```

### 기준별 키 인코딩
각 기준 `SortSpec`에 대해 한 행의 컬럼 값 → u64 키:
- 문자: 앞 8바이트 big-endian pack(기존 `text_key`).
- 숫자: 정렬가능 u64(기존 `f64_sortable`), 비수치=NUM_INVALID(맨 뒤).
- **방향**: `Asc`면 그대로, `Desc`면 `!key`(비트 반전). 비트 반전은 정수 순서를
  완전히 뒤집으므로, 오름 배열 비교 하나로 내림이 동시에 표현된다.
  - 주의: 숫자 비수치(NUM_INVALID=u64::MAX)를 Desc에서 `!MAX = 0`으로 반전하면
    "맨 앞"이 되어 비수치 뒤로 정책이 깨진다. → **방향 인코딩은 유효 키에만**
    적용하고, 비수치는 방향 무관하게 항상 맨 뒤가 되도록 별도 처리(아래 참조).

### 비수치 처리(다중 기준)
숫자 기준에서 비수치는 그 기준 위치에서 항상 맨 뒤여야 한다. 단일 컬럼에선
Desc일 때 유효 키만 reverse했지만, 다중 키 배열에선 그 방식이 안 통한다.
해결: 비수치 키를 방향과 무관하게 **항상 최댓값(u64::MAX)** 으로 두고, Desc
유효 키는 `!key`로 반전하되 **비수치는 반전하지 않는다**. 유효 Desc 키의 최댓값은
f64_sortable 특성상 `!(0xFFF0..) = 0x000F..` 부근이라 u64::MAX(비수치)보다 작아
비수치가 뒤에 남는다. (유효 키가 반전돼도 u64::MAX에 도달하지 못하므로 안전.)

## 데이터 흐름

1. 툴바 "다중 컬럼 정렬" 클릭 → 모달 창 오픈(`show_multi_sort_dialog` 플래그).
2. 모달: 기준 행 리스트. 각 행 = [컬럼 ComboBox][문자/숫자 ComboBox][오름/내림
   ComboBox][삭제 버튼]. "+ 기준 추가"(MAX_KEYS 미만일 때), "정렬"/"취소".
3. "정렬" → `spawn_multi_sort`로 백그라운드 시작(단일과 동일한 SortJob 인프라).
4. 키 추출: 각 행에서 기준마다 컬럼 키를 뽑아 `[u64; 4]` 구성(미사용 0 패딩).
5. `par_sort_unstable_by`로 `keys.cmp()` + row tie-break.
6. permutation → SortState. 렌더는 기존과 동일(permutation 경유).

## 파일 구조

### 수정: src/sort.rs
- `SortSpec { col, kind, dir }` 공개 구조체.
- `MAX_KEYS: usize = 4`, `MultiKey = [u64; 4]`.
- `col_key(field_bytes, kind, dir) -> u64`: 단일 컬럼 값 → 방향 반영 u64 키
  (기존 text_key/number_key + 방향 반전, 비수치 예외).
- `multi_key_for_row(...) -> [u64; 4]`: 한 행에서 specs 순서대로 키 배열 생성.
- `extract_and_multi_sort(source, index, enc, delim, specs, data_start, progress)
  -> Vec<u32>`: 단일 extract_and_sort의 다중 버전. snapshot + par_chunks_mut로
  키 배열 추출, par_sort_unstable_by(keys then row).
- `spawn_multi_sort(...) -> SortJob`: 백그라운드 래퍼(기존 spawn_sort와 대칭).
- 기존 단일 경로(extract_and_sort/spawn_sort)는 유지(단일 컬럼 정렬 그대로).

### 수정: src/app.rs
- `App`(또는 Document)에 `show_sort_dialog: bool`, `sort_specs: Vec<SortSpec>`.
- 툴바 정렬 컨트롤에 "다중 정렬…" 버튼 → 다이얼로그 토글.
- `egui::Window`로 모달 다이얼로그. 기준 행 편집 UI. col_count는 render에서 이미
  계산하므로 전달.
- "정렬" 클릭 시 spawn_multi_sort → sort_job. SortState는 다중임을 표시(헤더
  화살표는 1차 기준 컬럼에만, 상태바에 "N개 기준 정렬됨").
- 기존 무효화(구분자/인코딩/헤더 변경, 재인덱싱)에 sort_specs/dialog도 리셋.

## 엣지 케이스
- 기준 0개로 "정렬" → 무시(또는 버튼 비활성).
- 같은 컬럼 중복 지정 허용(무의미하지만 막지 않음 — 2차가 사실상 안 쓰임).
- 인덱싱 미완료면 다중 정렬도 비활성(단일과 동일).
- 텍스트 모드(구분자 None)에선 다중 정렬 비활성.
- MAX_KEYS(4) 초과 추가 불가.
- SortState는 단일/다중 공용: `col`(1차 기준 컬럼), 화살표는 1차만.

## 테스트 전략
sort.rs 순수 함수 단위 테스트:
1. 2기준(1차 문자 오름, 2차 숫자 오름): 도시별→나이순 permutation 검증.
2. 방향 혼합(1차 오름, 2차 내림): 키 반전이 올바른 순서를 내는지.
3. 1차 동률 → 2차로 갈림(1차 같은 값 여러 행이 2차 기준으로 정렬).
4. 숫자 기준 비수치가 그 기준에서 맨 뒤(오름/내림 무관).
5. 기준 1개면 단일 정렬과 동일 결과(회귀).
6. 미사용 슬롯 0 패딩이 결과에 영향 없음.

## 비목표 (YAGNI)
- 4개 초과 기준.
- 다중 기준 각각의 8B 초과 tie-break 원본 재접근(앞 8B로 단순화).
- 기준 드래그 재정렬(추가/삭제로 충분, 순서는 위→아래 고정).
- 정렬 기준 프리셋 저장.
