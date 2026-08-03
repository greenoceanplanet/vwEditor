# Parquet(GeoParquet) 읽기 전용 뷰어 — 설계

**날짜:** 2026-08-03
**상태:** 설계 승인 대기

## 목표

`.parquet` 파일을 드래그앤드롭과 File▸Open으로 열어 표로 본다. 읽기 전용이다.
GeoParquet의 geometry 컬럼은 요약 텍스트로 보여준다.

지원 기능: 찾기(Ctrl+F), 정렬/컬럼 선택, CSV/TSV로 내보내기.

## 왜 읽기 전용인가

Parquet은 컬럼 지향 + 압축 + 인코딩된 바이너리 포맷이다. 한 행이 파일 안 한 자리에
연속으로 있지 않고 컬럼별로 흩어져 있으며, 각 컬럼 청크는 dictionary/RLE/delta
인코딩을 거친 뒤 Snappy/ZSTD로 압축되고 페이지마다 체크섬과 통계가 붙는다.

셀 하나를 고치려면 해당 페이지를 압축 해제 → 디코드 → 값 변경 → 재인코딩 →
재압축해야 하고, 크기가 달라지므로 뒤따르는 모든 청크의 오프셋이 밀리고 푸터
메타데이터를 전부 재계산해야 한다. 사실상 파일 전체 재작성이다.

텍스트 파일에서 "수정분만 반영해 저장"이 가능한 것은 바이트 오프셋이 안정적이기
때문인데, Parquet에는 그 성질이 없다. 10GB 파일의 셀 하나를 고치는 것은 10GB를
다시 쓰는 일이다.

## 아키텍처

**Parquet 문서는 네 번째 문서 종류가 아니라, 표 문서의 세 번째 데이터 출처다.**

행을 꺼내는 통로 `logical_line`이 이미 편집 버퍼와 mmap 디코드를 가르고 있고,
호출부가 25곳이다. 여기에 갈래를 하나 더 넣으면 표 렌더링·찾기·내보내기가
따라온다:

```rust
pub fn logical_line(doc: &Document, logical: usize) -> Option<String> {
    if let Some(e) = &doc.edit { e.lines.get(logical).cloned() }
    else if let Some(p) = &doc.parquet { p.row_line(logical, RowScope::All) }
    else { decode_logical_line(doc, logical) }
}
```

근거:
- 찾기는 `search_from`이 `get_line` 클로저를 `find.rs`에 넘긴다. `find.rs`는
  Source도 mmap도 모른다 — 이미 추상화돼 있다.
- 내보내기는 `save::write_file(path, lines: &[String], opts, progress)`로
  포맷 무관이다.

**정렬만 이 이음매를 우회한다.** `sort.rs:320`과 `:485`가 `source.as_bytes()`로
mmap 원본 바이트를 직접 훑고 `field_slice`로 필드를 잘라낸다. Parquet에는 구분자로
나뉜 원본 텍스트가 존재하지 않으므로 별도 경로가 필요하다(아래 "정렬" 절).

### 채택하지 않은 대안

**행 공급자 트레이트 도입(`trait RowSource`).** 이론적으로 더 깔끔하고 문자열
왕복이 없지만, `render_table` 하나가 1200줄이고 mmap 바이트 슬라이스에 깊이
묶여 있다. 트레이트로 바꾸면 744개 테스트가 걸린 기존 텍스트 경로를 Parquet
때문에 전부 다시 배선해야 한다. 세 번째 포맷이 생길 때 하면 된다.

**Parquet 전용 렌더 경로(`render_parquet`, hex 방식).** 기존 코드를 전혀 안
건드리지만 찾기·정렬·내보내기·셀 선택·복사를 전부 새로 써야 한다. hex가 그래도
됐던 것은 기능이 거의 없어서였다.

**DuckDB 임베드.** 통계 기반 스킵(predicate pushdown)과 튜닝된 외부 정렬을
공짜로 얻지만, C++ 빌드로 컴파일 수 분·바이너리 수십 MB가 붙는다. 현재 직접
의존성 10개의 순수 Rust 프로젝트이고 목표가 "즉시 뜬다"이다. 게다가 스크롤은
DuckDB가 유리하지 않다 — "100만 행째부터 50행"은 row group 하나 푸는 것이 더
직접적이다. 조건 필터를 나중에 넣을 때도 row group 통계를 직접 읽으면 된다.

## 의존성 비용 (실측)

스크래치 프로젝트에서 실제로 재어 확인한 값이다. 추정이 아니다.

| 항목 | 값 |
|---|---|
| 추가되는 크레이트 | **52개** (parquet 47 + serde_json 5) |
| 콜드 릴리즈 빌드 | **37초** (parquet 스택만) |
| 최소 feature 조합 | `arrow`, `snap`, `zstd` |
| 요구 rustc | 1.85 (현재 1.96 — 충족) |

**`arrow` feature는 뺄 수 없다.** `parquet::arrow` 모듈 전체가 그 게이트 뒤에
있어서, 빼면 `ProjectionMask`와 `ParquetRecordBatchReaderBuilder`가 사라진다.
따라서 `arrow-ipc`와 `flatbuffers`도 함께 딸려 온다(읽기에는 안 쓰이지만 뺄 수
없다). 47개가 하한이다.

`default-features = false`로 `brotli`/`lz4`/`flate2`/`simdutf8`는 뺀다. Snappy와
ZSTD가 실무 Parquet의 대부분이다. 빠진 압축으로 인코딩된 파일은 "지원하지 않는
압축" 오류로 처리한다(아래 오류 처리).

빌드 37초는 **한 번만** 드는 비용이다 — 의존성이므로 이후 증분 빌드는 영향받지
않는다. DuckDB의 수 분 C++ 빌드와는 성격이 다르다.

### 검증된 API

스펙이 가정하는 API가 실제로 존재하고 컴파일되는 것을 프로브로 확인했다:

- `ParquetRecordBatchReaderBuilder::try_new(File)` — 푸터만 읽음
- `builder.metadata().file_metadata().num_rows()` — 행 수 즉시 확정
- `meta.row_group(i).num_rows() / .total_byte_size()` — row group 경계
- `file_metadata().key_value_metadata()` — `geo` 키 조회 (GeoParquet 감지)
- `ProjectionMask::roots(schema, [col_idx])` — 컬럼 프로젝션
- `.with_row_groups(vec![i]).with_projection(mask).with_batch_size(n)` —
  row group 단위 읽기 (LRU 캐시 단위)

## 파일 구조

**신규: `src/parquet.rs`** — egui 없는 순수 로직. `hex.rs`/`find.rs`/`convert.rs`와
같은 규율이다. `app.rs`는 이미 18k줄이라 더 늘리지 않는다.

담는 것:
- `ParquetDoc` — 메타데이터, row group LRU 캐시, 컬럼 이름
- 타입별 셀 포맷 (null/timestamp/decimal/중첩/binary)
- WKB 요약 파서
- 행 조립 (`row_line`)과 CSV 인용

**수정: `src/app.rs`**
- `Document`에 `parquet: Option<ParquetDoc>` 필드
- `open_path`에 PAR1 매직 분기, `open_path_parquet` 추가
- `logical_line`에 Parquet 갈래
- 정렬 경로 분기, 기능 게이트(편집/저장/변환 차단)

**수정: `Cargo.toml`** — 정확한 형태는 다음과 같다(프로브에서 컴파일 확인):

```toml
parquet = { version = "59", default-features = false, features = ["arrow", "snap", "zstd"] }
serde_json = "1"
```

`arrow`를 **별도 의존성으로 추가하지 않는다.** `parquet`의 `arrow` feature가
필요한 `arrow-*` 크레이트를 정확한 버전으로 끌어온다. 따로 적으면 버전이
어긋날 수 있다.

**수정 없음:** `find.rs`, `save.rs`, `source.rs`, `edit.rs`, `hex.rs`, `index.rs`,
`indexer.rs`

## 진입점

`open_path`가 이미 앞 `PRIME_BYTES`(64KB)를 읽고 `detect_text`로 분기한다
(`app.rs:551`). 그 앞에 매직 검사를 넣는다:

```rust
if head.starts_with(b"PAR1") {
    self.open_path_parquet(path);
    return;
}
```

**드래그앤드롭과 File▸Open이 둘 다 `open_path`로 모이므로 한 곳만 고치면 양쪽이
된다.** 드롭 경로는 `plan_dropped_files` → `open_path`이고, 메뉴는
`pick_file()` → `open_path`다.

Open 다이얼로그 필터에 추가:
```rust
.add_filter("Parquet", &["parquet"])
```

확장자가 아니라 매직으로 판단하므로 `.pq`처럼 다른 확장자도 열린다. 반대로
`.parquet` 확장자인데 내용이 Parquet이 아니면 기존 텍스트/바이너리 경로로 간다.

## 읽기 전략

세 층으로 나눈다.

### 열기 — 푸터만

`ParquetMetaData`만 읽어 행 수·스키마·row group 경계를 확정한다. 파일 크기와
무관하게 즉시다.

CSV처럼 개행을 세지 않으므로 `indexer.rs`의 백그라운드 인덱싱이 필요 없다.
**Parquet 문서에서 `LineIndex`는 쓰이지 않는다.** `doc.index`는 빈 상태로 두고
(`LineIndex::new(len)` 후 인덱서를 띄우지 않음), 행 수는 `ParquetDoc`이 답한다.

`doc_line_count`가 Parquet 문서에서 `total_rows + 1`을 돌려주도록 분기한다.

### `doc.source`는 그대로 mmap한다

`ParquetDoc`은 자체 `File` 핸들로 읽으므로 `doc.source`가 행 조회에 쓰이지
않는다. 그렇다고 `Option`으로 바꾸지 않는다 — `Document.source`는
`Arc<Source>`이고 참조가 20곳이 넘어, `Option`으로 만들면 무관한 코드를 전부
고쳐야 한다.

파일을 평범하게 mmap해서 넣는다. mmap은 지연 매핑이라 실제로 읽지 않으면
물리 메모리를 쓰지 않으므로 10GB 파일에서도 비용이 없고, 상태바의 파일 크기
표시(`d.source.len()`)가 공짜로 맞는다.

**바이트를 훑는 경로가 Parquet에서 도는 것을 막는 것은 빈 `LineIndex`다.**
`scan_all_matches`의 바이트 빠른 경로(`app.rs:3401`)는 `offsets.is_empty()`면
즉시 반환하므로 Parquet에서는 `doc.source.as_bytes()`에 도달하지 않는다.
다만 이것은 **우연히 안전한 것**이므로, Parquet 분기를 명시적으로 넣어 의도를
드러낸다(빈 인덱스에 의존하지 않는다).

### 렌더 — 보이는 컬럼만

화면에 보이는 컬럼 집합으로 `ProjectionMask`를 만들어 그 컬럼만 압축 해제한다.
100개 중 8개가 보이면 디코드 비용이 1/12이다. 컬럼 지향의 핵심 이점이고 스크롤
체감을 좌우한다.

### 캐시 — row group LRU

디코드된 row group을 `Vec<Vec<String>>`(행 × 열 문자열)로 들고 최대 4개 유지한다.

**캐시 키는 `(row group 인덱스, 컬럼 집합)`이다.** 컬럼 집합을 키에 넣지 않으면,
가로 스크롤로 보이는 컬럼이 바뀌었을 때 예전 컬럼 집합으로 디코드된 캐시가 그대로
쓰여 빈 셀이 나온다. 컬럼 집합이 바뀌면 그 그룹을 다시 디코드한다.

### 용도별 컬럼 집합

렌더가 보이는 컬럼만 읽는 것과 찾기가 전체를 봐야 하는 것이 부딪힌다. 안 보이는
컬럼을 비워 두면 찾기가 그 컬럼의 매치를 놓친다.

```rust
pub enum RowScope {
    /// 렌더용 — 보이는 컬럼만.
    Visible(ProjectionMask),
    /// 찾기·내보내기·정렬용 — 전체 컬럼.
    All,
}
```

**찾기는 캐시를 오염시키지 않는다.** 전체 스캔이라 캐시가 의미 없고, LRU를 밀어내면
스크롤이 느려진다. 별도로 row group을 순차 스트리밍한다.

### 찾기 배선 — 코드에서 확인한 정확한 지점

`scan_all_matches`는 `doc.edit`로 갈린다: `Some`이면 `e.lines` 순회, `None`이면
mmap 바이트 빠른 경로 3종(`scan_view_memmem` / `scan_view_ci_bytes` /
`scan_rows_scoped`)이다. Parquet은 어느 쪽도 아니므로 **분기를 하나 더 넣는다.**

행 단위 폴백 `scan_rows_scoped`(`app.rs:3496`)가 이미 `logical_line`을 쓰므로
구조를 그대로 재사용할 수 있다. 그러나 **그대로 부르면 안 된다:**

```rust
let n = doc.index.line_count();   // Parquet은 빈 인덱스라 0 → 아무것도 못 찾는다
```

Parquet 문서에서 `LineIndex`는 비어 있으므로 `n = 0`이 되어 **조용히 0건을
반환한다.** 오류도 안 나고 테스트도 "찾기가 동작한다"만 보면 통과한다.

따라서 `scan_rows_scoped`가 `doc.index.line_count()`가 아니라
`doc_line_count(doc)`를 쓰도록 고친다. `doc_line_count`는 편집 버퍼와 Parquet을
모두 아는 유일한 함수이고, 텍스트 뷰 모드에서는 `doc.index.line_count()`와 같은
값이라 **기존 동작이 바뀌지 않는다.**

같은 이유로 `search_from`(다음/이전 찾기)은 이미 `doc_line_count`를 쓰므로
수정이 필요 없다.

**검색 순서.** Parquet 분기는 바이트 빠른 경로들보다 **먼저** 판정한다. 그것들은
mmap 바이트를 전제하므로 Parquet에서 성립하지 않는다.

## 행 조립과 이스케이프

`row_line`이 셀을 구분자로 이어 붙이면 표는 그것을 `split_fields`로 다시 자른다.
값 안에 구분자나 따옴표나 개행이 있으면 컬럼이 어긋난다.

**이어 붙일 때 CSV 규칙으로 정확히 인용한다.** 값에 구분자·`"`·CR·LF가 있으면
전체를 `"`로 감싸고 내부 `"`는 `""`로 바꾼다.

**이 규칙이 실제로 왕복하는 것을 프로브로 확인했다.** `split_fields`의 본체를
그대로 떼어내 14가지 입력(구분자 포함, 따옴표 포함, 따옴표로 시작, 한글, LF,
CRLF, CR 단독, 탭, 앞뒤 공백, `"""`, geometry 요약 등)을 넣어 전부 복원되는 것을
확인했다. `csv_core`가 인용 안의 개행도 정확히 처리한다.

### 개행은 인용만으로 부족하다 — 반드시 치환한다

왕복은 성립하지만 **화면이 깨진다.** 텍스트 경로는 `decode_logical_line`이
`trim_end_matches(['\r','\n'])`로 개행을 제거하므로 한 줄에 개행이 **절대**
없다는 것이 표 렌더링의 전제다. Parquet 문자열 셀에는 개행이 들어갈 수 있고,
그대로 두면 egui가 여러 줄 galley를 만들어 행 높이와 정렬이 어긋난다.

따라서 **셀 값의 CR/LF를 표시 단계에서 공백으로 바꾼다.** 인용보다 먼저 적용한다:

- `\r\n` → 공백 하나
- 단독 `\r`, 단독 `\n` → 공백 하나

치환 후에는 값에 개행이 없으므로 인용은 구분자와 따옴표만 신경 쓰면 된다.

**이것은 손실 변환이다.** 읽기 전용 뷰어이므로 원본이 훼손될 위험은 없지만,
CSV로 내보내면 개행이 공백으로 바뀐 채 나간다. 스펙의 의도적 선택이다 — 대안
(개행을 살려 내보내기)은 화면과 내보내기 결과가 달라져 더 혼란스럽다.

**구분자는 콤마 고정이다.** Parquet에는 원본 구분자라는 개념이 없다. 사용자가
툴바에서 바꿀 수 있게 하면 값에 그 문자가 들어갈 때 재인용이 필요해진다.
`doc.sep`는 `SeparatorMode::Char(b',')`로 고정하고 툴바의 구분자 선택은 Parquet
문서에서 비활성화한다. 내보내기에서만 대상 구분자를 고른다.

**헤더.** `doc.has_header = true`. 첫 논리 행(인덱스 0)이 컬럼 이름 행이고,
데이터 행은 1부터다. 이렇게 하면 `render_table`의 `data_start` 계산이 텍스트
경로와 동일하게 동작한다.

이것이 인덱스 규약을 만든다. **혼동하면 데이터가 한 행씩 밀리므로 명시한다:**

| 이름 | 뜻 | 값 |
|---|---|---|
| `total_rows` | 파일의 실제 데이터 행 수 (푸터에서) | N |
| `doc_line_count(doc)` | 논리 행 수 (헤더 포함) | N + 1 |
| `row_line(0)` | 컬럼 이름을 인용해 이어붙인 행 | 헤더 |
| `row_line(k)`, k ≥ 1 | 데이터 행 | 파일의 k-1번째 행 |

즉 `row_line(k)`는 내부에서 `k - 1`을 파일 행 번호로 쓴다. `doc_line_count`는
Parquet 문서에서 `total_rows + 1`을 돌려주도록 분기한다.

**컬럼 이름도 인용 규칙을 탄다.** 컬럼 이름에 쉼표나 따옴표가 들어갈 수 있으므로
(Parquet 스키마는 허용한다) 데이터 셀과 같은 함수로 인용한다.

**정렬 순열은 절대 논리 행번호를 담는다(헤더 포함 좌표계).** 이것은 코드에서
확인한 사실이다 — `app.rs:2759` 주석과 `app.rs:13824` 테스트가 명시한다:

```
permutation[view_row] = 논리 행번호   // view_row는 헤더를 뺀 0-based 화면 행
```

즉 헤더가 있으면 **값은 1부터 시작한다.** 길이는 `total_rows`(데이터 행 수)이고
담기는 값의 범위는 `1..=total_rows`다. 인덱스는 0-based 화면 행, 값은 1-based
논리 행이라는 **비대칭**이 핵심이고, 여기를 혼동하면 모든 행이 하나씩 밀린다.

Parquet 정렬이 순열을 만들 때: 파일 행 `f`(0-based)의 논리 행번호는 `f + 1`이다.
따라서 정렬 결과를 `permutation`에 넣을 때 **+1을 해야 한다.**

## 타입별 표시

**손으로 포맷하지 않는다. `arrow_cast::display::ArrayFormatter`를 쓴다.**

처음에는 타입별 포맷 규칙을 직접 표로 정하고 구현할 생각이었으나, 프로브로
확인해 보니 `ArrayFormatter`가 이미 원하던 결과를 정확히 낸다:

| 타입 | ArrayFormatter 출력 (실측) |
|---|---|
| int64 | `1`, `-42` |
| bool | `true` / `false` |
| float64 | `1.5`, `0.30000000000000004`, `1e20` |
| utf8 | `강남역` |
| timestamp(ms) | `2026-03-31T23:33:20` |
| timestamp+tz | `2026-04-01T08:33:20+09:00` |
| date32 | `2024-10-04` |
| decimal(10,2) | `1234.56`, `-0.05` |
| list&lt;utf8&gt; | `[a, b]` |

날짜·시각·decimal·타임존이 전부 스펙이 원한 형태다. 직접 구현하면 이것을
재현하려다 틀릴 뿐이다.

### 다만 두 가지는 덮어쓴다

**1. null → 빈 문자열.** `NULL`이라 쓰면 실제 문자열 "NULL"과 구분되지 않고,
CSV로 내보낼 때도 빈 값이 관행이다.

**정정:** 처음에 "`ArrayFormatter`가 null을 `<null>`로 낸다"고 적었으나 **틀렸다.**
기본 `FormatOptions`는 null을 이미 빈 문자열로 낸다(실측 확인). 그래도 `is_null`
분기를 명시적으로 두는 이유는, 그 동작이 옵션 기본값에 딸린 것이라 라이브러리가
바꾸거나 우리가 옵션을 손대면 조용히 마커가 새어 나오기 때문이다 — 계약을 코드에
박아 두고 테스트가 지킨다.

**2. binary → `<binary N B>`.** `ArrayFormatter`는 바이너리를 전체 16진수 덤프로
낸다(실측: 21바이트 WKB가 42자). 큰 값이면 셀이 감당하지 못한다. 바이너리
컬럼은 길이 요약으로 바꾸고, geometry 컬럼이면 WKB 요약으로 바꾼다.

중첩 타입(list/struct/map)은 `ArrayFormatter` 출력을 그대로 쓴다 — 원래
`[N items]`로 요약하려 했으나, 실제 내용을 보여주는 편이 뷰어로서 유용하고
셀 폭은 표가 이미 잘라낸다.

**null은 빈 문자열이다.** `NULL`이라 쓰면 실제 문자열 "NULL"과 구분되지 않고,
CSV로 내보낼 때도 빈 값이 관행이다.

**중첩 타입은 전개하지 않는다.** 표 뷰어의 한 셀에 들어갈 수 없고, 전개하면
컬럼 수가 동적으로 변해 표 구조가 무너진다.

## GeoParquet

GeoParquet은 평범한 Parquet이다. 파일 키-값 메타데이터에 `geo` 키가 있고, 값이
JSON이다. 거기에 어느 컬럼이 geometry인지 적혀 있다:

```json
{"version":"1.0.0","primary_column":"geometry",
 "columns":{"geometry":{"encoding":"WKB","geometry_types":["Polygon"]}}}
```

`geo` 키가 있으면 그 JSON에서 geometry 컬럼 집합을 뽑는다.

**JSON 파서를 직접 쓰지 않는다. `serde_json`을 추가한다.**

처음에는 "`columns` 객체의 키만 뽑는 최소 스캔"을 생각했으나 철회한다. 손으로
쓰면 중첩 객체·이스케이프된 따옴표(`"my\"col"`)·유니코드 이스케이프를 전부
직접 다뤄야 하고, 그중 하나만 틀려도 **엉뚱한 컬럼을 geometry로 오인해** 정상
문자열 컬럼이 `<binary>`로 표시된다. 검증되지 않은 파서를 신뢰 경계에 두는 것은
비용 대비 위험이 크다.

`serde_json`은 실측 5개를 더한다(47 → 52). 이미 47개를 추가하는 마당에 5개를
아끼려고 신뢰 경계의 파서를 손으로 쓸 이유가 없다.

파싱 규칙:
- `columns` 객체의 각 키가 컬럼 이름
- 그 값의 `encoding`이 `"WKB"`인 것만 geometry로 취급 (대소문자 무시)
- `geo` 키가 없거나, JSON이 깨졌거나, `columns`가 없으면 **geometry 컬럼 없음**
  으로 처리하고 조용히 넘어간다. 오류로 열기를 실패시키지 않는다 — geometry
  표시는 부가 기능이고, 그것 때문에 파일을 못 여는 것이 더 나쁘다.
- JSON에 있는 컬럼 이름이 실제 스키마에 없으면 무시한다.

### WKB 요약 파서

의존성 없이 직접 쓴다. 헤더만 읽는다:

```
바이트 0    : 엔디안 (0 = big, 1 = little)
바이트 1..5 : geometry 타입 (u32)
이후        : 타입별 좌표/파트 수
```

타입 코드와 표시:

| 코드 | 타입 | 표시 |
|---|---|---|
| 1 | Point | `POINT(127.024 37.512)` |
| 2 | LineString | `LINESTRING(N pts)` |
| 3 | Polygon | `POLYGON(N pts)` — 모든 링의 좌표 합 |
| 4 | MultiPoint | `MULTIPOINT(N pts)` |
| 5 | MultiLineString | `MULTILINESTRING(N parts)` |
| 6 | MultiPolygon | `MULTIPOLYGON(N parts)` |
| 7 | GeometryCollection | `GEOMETRYCOLLECTION(N parts)` |

**Point만 좌표를 실제로 읽는다.** 나머지는 개수만 센다 — 큰 폴리곤에서 수만 개
좌표를 읽을 이유가 없다. 개수를 세는 것도 좌표를 건너뛰며 세지, 값을 파싱하지
않는다.

천 단위 구분 쉼표를 넣는다(`POLYGON(1,204 pts)`). 셀 값이 이미 인용 규칙을 타므로
쉼표가 컬럼을 깨지 않는다.

**폴백.** `geo` 메타데이터가 없거나, WKB 바이트가 너무 짧거나, 타입 코드가 1~7
밖이면 조용히 `<binary N B>`로 돌아간다. **뷰어가 데이터 문제로 죽으면 안 된다.**
Z/M 차원(코드 1000/2000/3000 오프셋)도 기본 타입으로 환원해 표시한다.

## 정렬

mmap 바이트 스캔이 불가능하므로 별도 경로다.

1. 정렬 키 컬럼**만** 프로젝션으로 읽는다 (다른 컬럼은 건드리지 않음)
2. `Vec<(key, u32 원본_행번호)>`를 만들어 rayon으로 정렬
3. 결과 순열을 기존 `SortState.permutation`에 넣는다

**3번이 핵심이다.** 순열이 만들어지면 렌더는 텍스트 경로와 완전히 동일해진다.
`render_table`은 이미 `permutation`으로 행을 매핑한다.

**컬럼 인덱스 규약.** `SortState.col`은 표의 컬럼 인덱스이고, Parquet 스키마의
컬럼 순서와 1:1로 같다(헤더 규약은 **행**에만 적용되고 열에는 영향이 없다).
따라서 `ProjectionMask::roots(schema, [col])`에 그대로 넘긴다.

### 메모리 게이트

**숫자 키는 행당 12바이트다**(키 8 + 순열 4). 1억 행이면 약 1.2GB.

**문자열 키는 다르다.** 길이가 가변이라 12바이트로 계산할 수 없다. 문자열
컬럼으로 정렬할 때는 실제 문자열을 들고 있어야 하므로 메모리가 데이터에 달렸다.
그래서 예상치를 다음과 같이 잡는다:

- 숫자/날짜/불리언 키: `행수 × 12바이트`
- 문자열 키: `행수 × (12 + 평균길이)`. 평균 길이는 **첫 row group의 해당 컬럼
  압축 해제 크기 ÷ 그 그룹의 행 수**로 추정한다(푸터가 아니라 실제 한 그룹을
  읽어 재는 값이라 대표성이 있다).

`PARQUET_SORT_CONFIRM_ROWS`가 아니라 **`PARQUET_SORT_CONFIRM_BYTES`로 판단한다** —
행 수가 아니라 예상 메모리가 실제 위험이기 때문이다. 기준값은 hex와 같은
**512MB**로 두고 상수를 공유하지 않고 별도로 정의한다(용도가 다르므로 한쪽을
조정할 때 다른 쪽이 따라 움직이면 안 된다).

게이트 방식은 hex 모드와 같다 — `confirm_sort` 플래그를 세우고 `tab_bar_locked`가
그동안 탭 조작을 막는다. 확인 전에는 정렬을 시작하지 않는다.

문구에 예상 메모리를 표시한다: "약 1.2GB 메모리를 씁니다. 계속하시겠습니까?"

### 정렬 키 타입

숫자 컬럼은 숫자로, 문자열 컬럼은 문자열로 정렬한다. Parquet은 타입이 확정적이라
텍스트 경로의 `SortKind` 추론(숫자로 보이는지 검사)이 필요 없다 — 스키마가 답을
갖고 있다.

## 기능 게이트

Parquet 문서에서 막는 것:

| 기능 | 처리 |
|---|---|
| 편집 모드 진입 | 툴바/메뉴 항목 비활성화 |
| `auto_edit_on_open` | **Parquet은 크기와 무관하게 진입하지 않는다** |
| 저장 / 덮어쓰기 | 비활성화 |
| 구분자 변환 | 비활성화 |
| 툴바 구분자 선택 | 비활성화 (콤마 고정) |
| 붙여넣기 / 셀 편집 | 무시 |
| 찾기 → 바꾸기 | 바꾸기만 비활성화, 찾기는 동작 |

**`auto_edit_on_open`이 특히 중요하다.** 현재 규칙은 크기 기준
(`size <= AUTO_EDIT_MAX_BYTES`)이라, 작은 Parquet 파일은 그대로 두면 자동으로
편집 모드에 들어간다. 명시적으로 막아야 한다.

### 게이트는 UI가 아니라 `enter_edit_mode` 안에 둔다

`enter_edit_mode`의 프로덕션 호출부는 세 곳이다 — `app.rs:681`(자동 진입),
`:1467`(메뉴), `:5383`. 각 호출부에서 막으면 **하나를 빠뜨리기 쉽고 새 호출부가
생기면 또 뚫린다.**

대신 함수 진입부에 가드를 넣는다. 이미 같은 규율의 선례가 있다:

```rust
pub fn enter_edit_mode(doc: &mut Document) {
    if doc.edit.is_some() { return; }        // 기존
    if doc.parquet.is_some() { return; }     // 추가 — 한 곳이 모든 경로를 막는다
    ...
}
```

UI 비활성화는 그 위에 얹는 **표시**일 뿐이다(버튼이 왜 안 되는지 보여주는 것).
실제 방어는 이 가드다. 테스트도 UI가 아니라 이 함수를 직접 불러 확인한다.

`load_edit_buffer`가 mmap 전체를 문자열로 올리는 함수이므로, 이 가드가 없으면
Parquet 바이너리가 깨진 문자열로 편집 버퍼에 들어간다.

허용하는 것: 스크롤, 셀 선택, 복사, 컬럼 선택, 정렬, 찾기, 다른 이름으로
내보내기(CSV/TSV).

## 내보내기

"다른 이름으로 저장"이 Parquet 문서에서는 "CSV/TSV로 내보내기"가 된다.

`logical_line(doc, i)`로 행을 순회하며 `Vec<String>`을 만들어
`save::write_file`에 넘긴다. 기존 저장 경로를 그대로 쓴다 — 인코딩·개행·BOM
선택도 따라온다.

`RowScope::All`로 읽으므로 안 보이는 컬럼도 전부 들어간다.

**진행률.** `write_file`이 이미 `progress: Option<&dyn Fn(usize)>`를 받는다.
Parquet은 디코드 비용이 있으므로 이 콜백을 반드시 연결한다.

정렬이 적용돼 있으면 정렬된 순서로 내보낸다(화면과 일치).

## 오류 처리

| 상황 | 처리 |
|---|---|
| 손상된 푸터 / PAR1은 맞지만 파싱 실패 | `self.error`에 메시지, 탭 안 열림 |
| 지원하지 않는 압축 (LZO 등) | 어떤 압축인지 밝힌 메시지, 탭 안 열림 |
| row group 디코드 실패 | 그 그룹의 셀만 `<error>` 표시, 앱은 계속 |
| 0행 Parquet | 헤더만 표시, 정상 |
| 컬럼 0개 | 빈 표, 정상 |
| WKB 파싱 실패 | `<binary N B>` 폴백 |

기존 규율을 따른다: 열기 실패는 `self.error`를 채우고 **탭을 추가하지 않는다**
(기존 탭은 그대로).

## 테스트 전략

기존 규율대로 순수 로직은 `parquet.rs`에서, 배선은 `app.rs`에서 테스트한다.

**`parquet.rs` 단위 테스트:**
- 타입별 셀 포맷 (null, timestamp, decimal, 중첩, binary)
- WKB 요약 — 타입 7종 + 깨진 바이트 + 짧은 바이트 + Z/M 차원
- CSV 인용 왕복: 셀 값에 구분자·따옴표·개행을 넣고
  `split_fields(row_line(i)) == 원래 셀들` 확인 — **이게 가장 중요한 테스트다**
- geo 메타데이터에서 geometry 컬럼 추출 (있음/없음/WKB 아닌 인코딩)
- LRU 캐시 — 컬럼 집합이 다르면 재디코드하는지

**`app.rs` 배선 테스트:**
- PAR1 매직으로 Parquet 경로를 타는지 (드롭·메뉴 양쪽)
- `.parquet` 확장자인데 내용이 텍스트면 텍스트로 열리는지
- `auto_edit_on_open`이 작은 Parquet에서 동작하지 않는지
- 편집/저장/변환이 실제로 차단되는지 (게이트가 비활성화만이 아니라
  호출돼도 무시하는지)
- 찾기가 안 보이는 컬럼의 매치도 잡는지 (`RowScope::All` 배선 확인)
- 정렬 순열이 실제 행 순서를 바꾸는지
- 정렬 게이트가 임계 초과에서 뜨는지

**테스트 픽스처.** `parquet` 크레이트의 writer로 테스트 시작 시 임시 파일을
생성한다. 바이너리를 저장소에 넣지 않는다. geometry 테스트는 WKB 바이트를 손으로
구성한다(Point는 21바이트로 짧다).

## 성능 목표

| 작업 | 목표 |
|---|---|
| 열기 (10GB) | 즉시 (푸터만) |
| 스크롤 한 화면 | row group 하나 디코드 이내 |
| 가로 스크롤 | 새 컬럼 집합 디코드 1회 |
| 찾기 (전체) | 전체 압축 해제 — CSV보다 느린 것이 정상 |
| 정렬 | 키 컬럼만 읽기 |

**스크롤이 끊긴다면 원인은 row group 디코드이지 문자열 할당이 아니다.** 가상
스크롤이라 한 프레임에 조립하는 행은 화면에 보이는 40~60줄뿐이다.

## 범위 밖 (나중에)

- 쓰기 / 편집 (구조적으로 불가)
- 조건 필터 (row group 통계 기반 스킵) — 넣는다면 별도 설계
- 중첩 타입 전개
- geometry 지도 표시
- 여러 Parquet 파일을 하나의 데이터셋으로 (파티션 디렉터리)
- Parquet으로 내보내기
