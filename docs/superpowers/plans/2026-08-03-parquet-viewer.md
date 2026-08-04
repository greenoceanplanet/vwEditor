# Parquet(GeoParquet) 읽기 전용 뷰어 구현 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `.parquet` 파일을 드래그앤드롭과 File▸Open으로 열어 표로 보고, 찾기·정렬·CSV 내보내기를 지원한다. 읽기 전용이다.

**Architecture:** Parquet 문서는 새 문서 종류가 아니라 **표 문서의 세 번째 데이터 출처**다. `logical_line(doc, i) -> Option<String>`이 이미 편집 버퍼와 mmap 디코드를 가르므로 갈래를 하나 더 넣으면 표 렌더링·찾기·내보내기가 따라온다. 순수 로직은 새 파일 `src/parquet.rs`에 두고(egui 없음, `hex.rs`와 같은 규율), `app.rs`에는 배선만 넣는다.

**Tech Stack:** Rust 2021, egui/eframe 0.28.1, `parquet` 59 (feature: arrow/snap/zstd), `serde_json` 1, rayon

## Global Constraints

- **읽기 전용이다.** Parquet 문서에서 편집·저장·구분자 변환·셀 편집·붙여넣기는 전부 막는다.
- **의존성은 정확히 이 형태다** (프로브에서 컴파일 확인):
  ```toml
  parquet = { version = "59", default-features = false, features = ["arrow", "snap", "zstd"] }
  serde_json = "1"
  ```
  `arrow`를 **별도 의존성으로 추가하지 않는다** — `parquet`의 `arrow` feature가 정확한 버전으로 끌어온다.
- **주석과 커밋 메시지는 한국어로 쓴다** (기존 코드베이스 관행).
- **`src/parquet.rs`에 egui를 import하지 않는다.** 순수 로직만.
- **인덱스 규약(틀리면 모든 행이 밀린다):**
  - `total_rows` = 파일의 실제 데이터 행 수
  - `doc_line_count(doc)` = `total_rows + 1` (헤더 포함)
  - `row_line(0)` = 컬럼 이름 행, `row_line(k)` (k ≥ 1) = 파일의 `k-1`번째 행
- **`SortState.permutation` 규약:** `permutation[view_row] = 논리 행번호`. 인덱스는 0-based 화면 행, **값은 1-based 논리 행**(헤더 포함 좌표계). 파일 행 `f`의 논리 행번호는 `f + 1`이다. (`app.rs:2759` 주석, `app.rs:13824` 테스트 참조)
- **셀 값의 CR/LF는 공백으로 치환한다.** 표 렌더링은 "한 줄에 개행 없음"을 전제한다.
- **구분자는 콤마 고정.** `doc.sep = SeparatorMode::Char(b',')`.
- 커밋 메시지 끝에 `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>` 를 넣는다.
- 각 태스크 종료 시 `cargo test` 전체 통과 + `cargo clippy` 경고 수가 기존(20) 이하.

---

## 파일 구조

| 파일 | 책임 |
|---|---|
| `src/parquet.rs` (신규) | 순수 로직 — 셀 포맷, WKB 요약, CSV 인용, 행 조립, geo 메타데이터 파싱, LRU 캐시, 메타데이터 로드 |
| `src/app.rs` (수정) | 배선 — `Document.parquet` 필드, `open_path` 매직 분기, `logical_line` 갈래, 찾기/정렬 분기, 기능 게이트 |
| `Cargo.toml` (수정) | 의존성 2개 추가 |

**수정하지 않는 파일:** `find.rs`, `save.rs`, `source.rs`, `edit.rs`, `hex.rs`, `index.rs`, `indexer.rs`, `parse.rs`, `sort.rs`, `theme.rs`

---

### Task 1: 의존성 추가와 테스트 픽스처

**Files:**
- Modify: `Cargo.toml`
- Create: `src/parquet.rs`
- Modify: `src/main.rs` (모듈 선언 추가)

**Interfaces:**
- Consumes: 없음 (첫 태스크)
- Produces: `crate::parquet` 모듈, 테스트 헬퍼 `#[cfg(test)] fn write_test_parquet(path: &Path, ...)`

이 태스크의 목적은 **의존성이 실제로 빌드되고 테스트에서 Parquet 파일을 만들 수 있음**을 확인하는 것이다. 이후 모든 태스크가 이 픽스처를 쓴다.

- [ ] **Step 1: `Cargo.toml`에 의존성 추가**

`[dependencies]` 섹션 끝에 추가한다:

```toml
# Parquet 읽기. `arrow`를 따로 적지 않는다 — parquet의 arrow feature가
# 맞는 버전의 arrow-* 크레이트를 끌어온다. 따로 적으면 버전이 어긋난다.
# default-features=false 로 brotli/lz4/flate2를 뺀다(snappy·zstd가 실무 대부분).
parquet = { version = "59", default-features = false, features = ["arrow", "snap", "zstd"] }
# geo 메타데이터(GeoParquet) JSON 파싱 전용.
serde_json = "1"
```

- [ ] **Step 2: 빌드 확인**

Run: `cargo build`
Expected: 성공. 첫 빌드는 약 40초 걸린다(크레이트 52개 추가).

- [ ] **Step 3: `src/main.rs`에 모듈 선언 추가**

기존 `mod hex;` 줄 근처에 알파벳 순서를 지켜 추가한다:

```rust
mod parquet;
```

- [ ] **Step 4: `src/parquet.rs` 생성 — 모듈 헤더와 테스트 픽스처**

```rust
//! Parquet 읽기 전용 뷰어의 순수 로직 — 셀 포맷, WKB 요약, 행 조립, 캐시.
//! egui 없음. `hex.rs`/`find.rs`/`convert.rs`와 같은 규율이다.

#[cfg(test)]
pub(crate) mod testutil {
    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::metadata::KeyValue;
    use parquet::file::properties::WriterProperties;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// 테스트마다 고유한 임시 파일 경로. `source.rs`의 `temp_file_with`와 같은
    /// 규율 — 내용 길이로 이름을 지으면 병렬 테스트끼리 같은 파일을 잡는다.
    pub fn temp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_pq_{}_{}_{}.parquet", std::process::id(), tag, id));
        p
    }

    /// (id: int64, name: utf8) 두 컬럼짜리 Parquet을 쓴다. `names`의 None은 null.
    pub fn write_simple(path: &Path, ids: Vec<i64>, names: Vec<Option<&str>>) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let id_col: ArrayRef = Arc::new(Int64Array::from(ids));
        let name_col: ArrayRef = Arc::new(StringArray::from(names));
        let batch = RecordBatch::try_new(schema.clone(), vec![id_col, name_col]).unwrap();
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let f = std::fs::File::create(path).unwrap();
        let mut w = ArrowWriter::try_new(f, schema, Some(props)).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
    }

    /// geo 키-값 메타데이터를 붙인 Parquet. `geo_json`이 None이면 geo 키 없음.
    pub fn write_with_geo(path: &Path, wkb: Vec<&[u8]>, geo_json: Option<&str>) {
        use arrow_array::BinaryArray;
        let schema = Arc::new(Schema::new(vec![
            Field::new("geometry", DataType::Binary, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        let n = wkb.len();
        let geom: ArrayRef = Arc::new(BinaryArray::from(wkb));
        let names: ArrayRef =
            Arc::new(StringArray::from((0..n).map(|i| format!("r{i}")).collect::<Vec<_>>()));
        let batch = RecordBatch::try_new(schema.clone(), vec![geom, names]).unwrap();
        let mut b = WriterProperties::builder().set_compression(Compression::SNAPPY);
        if let Some(j) = geo_json {
            b = b.set_key_value_metadata(Some(vec![KeyValue::new("geo".into(), j.to_string())]));
        }
        let f = std::fs::File::create(path).unwrap();
        let mut w = ArrowWriter::try_new(f, schema, Some(b.build())).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;

    #[test]
    fn fixture_writes_a_readable_parquet_file() {
        let p = temp_path("fixture");
        write_simple(&p, vec![1, 2, 3], vec![Some("가"), None, Some("다")]);
        let bytes = std::fs::read(&p).unwrap();
        assert!(bytes.starts_with(b"PAR1"), "Parquet 매직으로 시작해야 한다");
        assert!(bytes.ends_with(b"PAR1"), "Parquet은 매직으로 끝나기도 한다");
        let _ = std::fs::remove_file(&p);
    }
}
```

**주의:** 테스트 픽스처가 `arrow_array` / `arrow_schema`를 직접 import한다. 이들은 `parquet`의 `arrow` feature가 끌어오지만 **직접 이름으로는 접근할 수 없다.** `Cargo.toml`의 `[dev-dependencies]`에 추가한다:

```toml
[dev-dependencies]
arrow-array = "59"
arrow-schema = "59"
```

- [ ] **Step 5: 테스트 실행**

Run: `cargo test parquet::tests::fixture_writes_a_readable_parquet_file`
Expected: PASS

- [ ] **Step 6: 전체 테스트와 clippy**

Run: `cargo test`
Expected: 745개 통과 (기존 744 + 1)

Run: `cargo clippy 2>&1 | grep -c "^warning"`
Expected: 20 이하

- [ ] **Step 7: 커밋**

```bash
git add Cargo.toml Cargo.lock src/parquet.rs src/main.rs
git commit -m "chore: parquet/serde_json 의존성과 테스트 픽스처

Parquet 뷰어의 토대. 이 커밋은 기능이 없고 의존성이 실제로 빌드되는지와
테스트에서 Parquet 파일을 만들 수 있는지만 확인한다.

arrow를 별도 의존성으로 적지 않는다 - parquet의 arrow feature가 맞는
버전을 끌어온다. 따로 적으면 버전이 어긋난다.
default-features=false로 brotli/lz4/flate2를 뺐다(snappy/zstd가 실무 대부분).

크레이트 52개 추가, 콜드 빌드 약 40초.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: WKB 요약 파서

**Files:**
- Modify: `src/parquet.rs`

**Interfaces:**
- Consumes: Task 1의 모듈
- Produces: `pub fn wkb_summary(bytes: &[u8]) -> Option<String>`, `pub fn group_digits(n: u64) -> String`

geometry 컬럼 표시의 핵심이다. **헤더만 읽고 좌표는 개수만 센다** — 큰 폴리곤에서 수만 개 좌표를 파싱할 이유가 없다.

WKB 구조:
```
바이트 0    : 엔디안 (0 = big, 1 = little)
바이트 1..5 : geometry 타입 (u32)
이후        : 타입별 페이로드
```

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`src/parquet.rs`의 `mod tests`에 추가:

```rust
    /// little-endian POINT WKB를 만든다.
    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut v = vec![1u8, 1, 0, 0, 0];
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v
    }

    #[test]
    fn wkb_point_shows_coordinates() {
        let b = wkb_point(127.024, 37.512);
        assert_eq!(super::wkb_summary(&b).as_deref(), Some("POINT(127.024 37.512)"));
    }

    #[test]
    fn wkb_big_endian_point_is_read_correctly() {
        let mut v = vec![0u8, 0, 0, 0, 1];
        v.extend_from_slice(&1.5f64.to_be_bytes());
        v.extend_from_slice(&2.5f64.to_be_bytes());
        assert_eq!(super::wkb_summary(&v).as_deref(), Some("POINT(1.5 2.5)"));
    }

    #[test]
    fn wkb_polygon_counts_all_ring_points() {
        // 링 2개(4점 + 3점) → 7점
        let mut v = vec![1u8, 3, 0, 0, 0];
        v.extend_from_slice(&2u32.to_le_bytes());
        for n in [4u32, 3] {
            v.extend_from_slice(&n.to_le_bytes());
            for i in 0..n {
                v.extend_from_slice(&(i as f64).to_le_bytes());
                v.extend_from_slice(&(i as f64).to_le_bytes());
            }
        }
        assert_eq!(super::wkb_summary(&v).as_deref(), Some("POLYGON(7 pts)"));
    }

    #[test]
    fn wkb_z_dimension_falls_back_to_base_type() {
        // 1001 = Point Z. 기본 타입(1)으로 환원해 표시한다.
        let mut v = vec![1u8];
        v.extend_from_slice(&1001u32.to_le_bytes());
        v.extend_from_slice(&1.0f64.to_le_bytes());
        v.extend_from_slice(&2.0f64.to_le_bytes());
        assert_eq!(super::wkb_summary(&v).as_deref(), Some("POINT(1 2)"));
    }

    #[test]
    fn wkb_multipolygon_counts_parts_not_points() {
        let mut v = vec![1u8, 6, 0, 0, 0];
        v.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(super::wkb_summary(&v).as_deref(), Some("MULTIPOLYGON(3 parts)"));
    }

    #[test]
    fn wkb_rejects_broken_input_instead_of_panicking() {
        assert_eq!(super::wkb_summary(&[]), None, "빈 입력");
        assert_eq!(super::wkb_summary(&[1, 1]), None, "너무 짧음");
        assert_eq!(super::wkb_summary(&[9, 1, 0, 0, 0]), None, "엔디안 코드 이상");
        assert_eq!(super::wkb_summary(&[1, 99, 0, 0, 0]), None, "타입 코드 범위 밖");
        // POINT인데 좌표가 없다 — 패닉하지 않고 None
        assert_eq!(super::wkb_summary(&[1, 1, 0, 0, 0]), None, "좌표 부족");
    }

    #[test]
    fn group_digits_inserts_thousand_separators() {
        assert_eq!(super::group_digits(0), "0");
        assert_eq!(super::group_digits(999), "999");
        assert_eq!(super::group_digits(1204), "1,204");
        assert_eq!(super::group_digits(1_000_000), "1,000,000");
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests::wkb`
Expected: FAIL — `cannot find function 'wkb_summary'`

- [ ] **Step 3: 구현한다**

`src/parquet.rs`의 `#[cfg(test)] mod testutil` **위에** 추가한다:

```rust
/// 천 단위 쉼표를 넣는다. 셀 값은 인용 규칙을 타므로 쉼표가 컬럼을 깨지 않는다.
pub fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// WKB(Well-Known Binary) 한 값의 **요약**. 좌표를 전부 파싱하지 않는다 —
/// Point만 좌표를 읽고 나머지는 개수만 센다(큰 폴리곤에서 수만 개를 읽을
/// 이유가 없다).
///
/// 깨진 입력·짧은 입력·모르는 타입은 **None**이다. 호출부가 `<binary N B>`로
/// 폴백한다 — 뷰어가 데이터 문제로 죽으면 안 된다.
///
/// Z/M 차원(타입 코드 1000/2000/3000 오프셋)은 기본 타입으로 환원해 표시한다.
pub fn wkb_summary(b: &[u8]) -> Option<String> {
    if b.len() < 5 {
        return None;
    }
    let little = match b[0] {
        0 => false,
        1 => true,
        _ => return None,
    };
    let u32_at = |o: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    };
    let f64_at = |o: usize| -> Option<f64> {
        let s: [u8; 8] = b.get(o..o + 8)?.try_into().ok()?;
        Some(if little { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    };

    // Z/M 차원은 1000/2000/3000을 더해 표현한다(예: 1001 = Point Z).
    match u32_at(1)? % 1000 {
        1 => {
            let x = f64_at(5)?;
            let y = f64_at(13)?;
            Some(format!("POINT({x} {y})"))
        }
        2 => Some(format!("LINESTRING({} pts)", group_digits(u32_at(5)? as u64))),
        3 => {
            // 링마다 좌표 수가 앞에 붙는다. 좌표는 건너뛰며 개수만 더한다.
            let nrings = u32_at(5)? as usize;
            let mut off = 9usize;
            let mut total = 0u64;
            for _ in 0..nrings {
                let n = u32_at(off)? as u64;
                total += n;
                // 좌표 하나 = f64 두 개 = 16바이트. 오버플로 방지로 checked 연산.
                off = off.checked_add(4)?.checked_add((n as usize).checked_mul(16)?)?;
            }
            Some(format!("POLYGON({} pts)", group_digits(total)))
        }
        4 => Some(format!("MULTIPOINT({} pts)", group_digits(u32_at(5)? as u64))),
        5 => Some(format!("MULTILINESTRING({} parts)", group_digits(u32_at(5)? as u64))),
        6 => Some(format!("MULTIPOLYGON({} parts)", group_digits(u32_at(5)? as u64))),
        7 => Some(format!("GEOMETRYCOLLECTION({} parts)", group_digits(u32_at(5)? as u64))),
        _ => None,
    }
}
```

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test parquet::tests`
Expected: 8개 전부 PASS

- [ ] **Step 5: 커밋**

```bash
git add src/parquet.rs
git commit -m "feat: WKB 요약 파서 - geometry를 좌표 전부 읽지 않고 요약

GeoParquet의 geometry 컬럼 표시용. POINT만 좌표를 실제로 읽고 나머지는
개수만 센다 - 큰 폴리곤에서 수만 개 좌표를 파싱할 이유가 없다.

깨진 입력에 패닉하지 않고 None을 돌려준다. 뷰어가 데이터 문제로 죽으면
안 되므로 호출부가 <binary N B>로 폴백한다. 빈 입력/짧은 입력/이상한
엔디안 코드/범위 밖 타입/좌표 부족을 전부 테스트로 막았다.

Z/M 차원(1000/2000/3000 오프셋)은 기본 타입으로 환원한다.
폴리곤 링 순회에 checked 연산을 써서 조작된 길이값으로 오버플로가
나지 않게 했다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: 셀 포맷과 CSV 인용

**Files:**
- Modify: `src/parquet.rs`

**Interfaces:**
- Consumes: Task 2의 `wkb_summary`, `group_digits`
- Produces:
  - `pub fn sanitize_cell(s: &str) -> String`
  - `pub fn quote_cell(s: &str, delim: u8) -> String`
  - `pub fn join_row(cells: &[String], delim: u8) -> String`
  - `pub fn format_binary_cell(bytes: &[u8], is_geometry: bool) -> String`

**이 태스크의 왕복 테스트가 설계 전체에서 가장 중요하다.** `join_row`로 만든 줄을 `parse::split_fields`가 원래 셀로 되돌리지 못하면 표의 모든 컬럼이 어긋난다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn sanitize_replaces_newlines_with_space() {
        // 표 렌더링은 "한 줄에 개행 없음"을 전제한다(decode_logical_line이
        // trim_end_matches로 보장하는 성질). Parquet 셀에는 들어갈 수 있다.
        assert_eq!(super::sanitize_cell("a\r\nb"), "a b", "CRLF는 공백 하나");
        assert_eq!(super::sanitize_cell("a\nb"), "a b");
        assert_eq!(super::sanitize_cell("a\rb"), "a b");
        assert_eq!(super::sanitize_cell("깨끗함"), "깨끗함", "개행 없으면 그대로");
    }

    #[test]
    fn quote_cell_only_quotes_when_needed() {
        assert_eq!(super::quote_cell("plain", b','), "plain");
        assert_eq!(super::quote_cell("a,b", b','), "\"a,b\"");
        assert_eq!(super::quote_cell("a\"b", b','), "\"a\"\"b\"");
        // 탭 구분자면 콤마는 인용 대상이 아니다
        assert_eq!(super::quote_cell("a,b", b'\t'), "a,b");
        assert_eq!(super::quote_cell("a\tb", b'\t'), "\"a\tb\"");
    }

    /// **설계의 핵심 계약**: join_row로 만든 줄을 split_fields가 원래 셀로
    /// 되돌린다. 이게 깨지면 표의 모든 컬럼이 어긋난다.
    #[test]
    fn join_row_round_trips_through_split_fields() {
        let cases: Vec<Vec<&str>> = vec![
            vec!["a", "b", "c"],
            vec!["", "b", ""],
            vec!["a,b", "c"],
            vec!["a\"b", "c"],
            vec!["\"quoted\"", "c"],
            vec!["a,\"b", "c"],
            vec!["강남역", "서초구"],
            vec!["a\tb", "c"],
            vec![" leading", "trailing "],
            vec!["POLYGON(1,204 pts)", "서초구"],
            vec!["\"\"\"", "c"],
        ];
        for cells in cases {
            let owned: Vec<String> = cells.iter().map(|s| s.to_string()).collect();
            let line = super::join_row(&owned, b',');
            let back = crate::parse::split_fields(&line, b',');
            assert_eq!(back, owned, "왕복 실패: {owned:?} → {line:?} → {back:?}");
        }
    }

    #[test]
    fn join_row_round_trips_with_tab_delimiter() {
        let cells = vec!["a,b".to_string(), "c\td".to_string(), "e".to_string()];
        let line = super::join_row(&cells, b'\t');
        assert_eq!(crate::parse::split_fields(&line, b'\t'), cells);
    }

    #[test]
    fn binary_cell_summarizes_length_when_not_geometry() {
        assert_eq!(super::format_binary_cell(&[1, 2, 3], false), "<binary 3 B>");
        assert_eq!(super::format_binary_cell(&[], false), "<binary 0 B>");
    }

    #[test]
    fn binary_cell_uses_wkb_summary_for_geometry() {
        let pt = wkb_point(1.5, 2.5);
        assert_eq!(super::format_binary_cell(&pt, true), "POINT(1.5 2.5)");
    }

    #[test]
    fn binary_cell_falls_back_to_length_when_wkb_is_broken() {
        // geometry 컬럼이어도 WKB가 깨졌으면 길이 요약으로 돌아간다.
        assert_eq!(super::format_binary_cell(&[1, 99, 0, 0, 0], true), "<binary 5 B>");
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests`
Expected: FAIL — `cannot find function 'sanitize_cell'` 등

- [ ] **Step 3: 구현한다**

```rust
/// 셀 값에서 개행을 없앤다. **표 렌더링의 전제를 지키는 함수다.**
///
/// 텍스트 경로는 `decode_logical_line`이 `trim_end_matches(['\r','\n'])`로
/// 개행을 제거하므로 한 줄에 개행이 절대 없다. Parquet 문자열 셀에는 들어갈
/// 수 있고, 그대로 두면 egui가 여러 줄 galley를 만들어 행 높이와 정렬이
/// 어긋난다.
///
/// **손실 변환이다.** 읽기 전용이라 원본은 안전하지만 CSV로 내보내면 개행이
/// 공백이 된 채 나간다 — 화면과 내보내기 결과를 일치시키는 의도적 선택이다.
pub fn sanitize_cell(s: &str) -> String {
    if !s.contains(['\r', '\n']) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // CRLF는 공백 하나로 접는다.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

/// CSV 규칙으로 인용한다. `parse::split_fields`(csv_core)가 되돌릴 수 있는
/// 형태여야 한다 — 이 왕복이 표의 컬럼 정렬을 지탱한다.
pub fn quote_cell(s: &str, delim: u8) -> String {
    let needs = s.bytes().any(|b| b == delim || b == b'"' || b == b'\r' || b == b'\n');
    if !needs {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push('"'); // `""`로 이스케이프
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// 셀들을 한 논리 행 문자열로 잇는다. 표는 이것을 `split_fields`로 되자른다.
pub fn join_row(cells: &[String], delim: u8) -> String {
    let d = delim as char;
    let mut out = String::new();
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            out.push(d);
        }
        out.push_str(&quote_cell(c, delim));
    }
    out
}

/// 바이너리 셀 표시. `ArrayFormatter`는 바이너리를 전체 16진수 덤프로 내므로
/// (21바이트 WKB가 42자) 셀이 감당하지 못한다. 길이 요약으로 바꾸고,
/// geometry 컬럼이면 WKB 요약을 시도한다.
pub fn format_binary_cell(bytes: &[u8], is_geometry: bool) -> String {
    if is_geometry {
        if let Some(s) = wkb_summary(bytes) {
            return s;
        }
        // WKB가 깨졌으면 조용히 길이 요약으로 폴백한다.
    }
    format!("<binary {} B>", bytes.len())
}
```

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test parquet::tests`
Expected: 전부 PASS (왕복 테스트 11케이스 포함)

- [ ] **Step 5: 커밋**

```bash
git add src/parquet.rs
git commit -m "feat: 셀 포맷과 CSV 인용 - split_fields 왕복을 계약으로 박음

행 조립의 핵심. join_row로 만든 줄을 parse::split_fields가 원래 셀로
되돌리지 못하면 표의 모든 컬럼이 어긋나므로, 왕복 자체를 테스트로 박았다
(구분자 포함, 따옴표 포함, 따옴표로 시작, 한글, 탭, 앞뒤 공백, \"\"\",
geometry 요약 등 11케이스 + 탭 구분자).

개행은 인용만으로 부족해 공백으로 치환한다. 텍스트 경로는
decode_logical_line이 개행을 제거하므로 \"한 줄에 개행 없음\"이 표
렌더링의 전제인데, Parquet 문자열 셀에는 들어갈 수 있다. 그대로 두면
egui가 여러 줄 galley를 만들어 행 정렬이 깨진다. 손실 변환이지만
읽기 전용이라 원본은 안전하다.

바이너리는 ArrayFormatter의 16진수 덤프(21바이트가 42자) 대신 길이
요약으로 바꾸고, geometry면 WKB 요약을 시도한 뒤 실패 시 폴백한다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: geo 메타데이터 파싱

**Files:**
- Modify: `src/parquet.rs`

**Interfaces:**
- Consumes: Task 1의 `testutil::write_with_geo`
- Produces: `pub fn geometry_columns(geo_json: Option<&str>) -> std::collections::HashSet<String>`

GeoParquet은 평범한 Parquet이고, 파일 키-값 메타데이터의 `geo` 키에 어느 컬럼이 geometry인지 JSON으로 적혀 있다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn geometry_columns_reads_wkb_columns_from_geo_json() {
        let j = r#"{"version":"1.0.0","primary_column":"geom",
                    "columns":{"geom":{"encoding":"WKB"}}}"#;
        let got = super::geometry_columns(Some(j));
        assert!(got.contains("geom"));
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn geometry_columns_accepts_multiple_columns() {
        let j = r#"{"columns":{"a":{"encoding":"WKB"},"b":{"encoding":"wkb"}}}"#;
        let got = super::geometry_columns(Some(j));
        assert!(got.contains("a"), "대문자 WKB");
        assert!(got.contains("b"), "소문자 wkb도 받는다");
    }

    #[test]
    fn geometry_columns_skips_non_wkb_encodings() {
        // WKT나 미래의 다른 인코딩은 WKB 파서로 읽을 수 없으므로 제외한다.
        let j = r#"{"columns":{"a":{"encoding":"WKT"},"b":{"encoding":"WKB"}}}"#;
        let got = super::geometry_columns(Some(j));
        assert!(!got.contains("a"), "WKT는 제외");
        assert!(got.contains("b"));
    }

    #[test]
    fn geometry_columns_is_empty_without_geo_metadata() {
        assert!(super::geometry_columns(None).is_empty(), "geo 키 없음");
    }

    #[test]
    fn geometry_columns_survives_broken_json() {
        // 깨진 JSON으로 파일 열기를 실패시키지 않는다 - geometry 표시는
        // 부가 기능이고, 그것 때문에 파일을 못 여는 것이 더 나쁘다.
        assert!(super::geometry_columns(Some("{not json")).is_empty());
        assert!(super::geometry_columns(Some("null")).is_empty());
        assert!(super::geometry_columns(Some("[]")).is_empty(), "배열");
        assert!(super::geometry_columns(Some("{}")).is_empty(), "columns 없음");
        assert!(
            super::geometry_columns(Some(r#"{"columns":"nope"}"#)).is_empty(),
            "columns가 객체가 아님"
        );
        assert!(
            super::geometry_columns(Some(r#"{"columns":{"a":5}}"#)).is_empty(),
            "컬럼 값이 객체가 아님"
        );
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests::geometry`
Expected: FAIL — `cannot find function 'geometry_columns'`

- [ ] **Step 3: 구현한다**

```rust
/// `geo` 키-값 메타데이터에서 **WKB로 인코딩된 geometry 컬럼 이름**을 뽑는다.
///
/// GeoParquet은 평범한 Parquet이고 `geo` 키에 JSON이 들어 있다:
/// `{"version":"1.0.0","primary_column":"geometry",
///   "columns":{"geometry":{"encoding":"WKB",...}}}`
///
/// **JSON을 손으로 파싱하지 않는다.** 이스케이프된 따옴표(`"my\"col"`) 하나만
/// 잘못 처리해도 멀쩡한 문자열 컬럼을 geometry로 오인해 `<binary>`로 표시한다.
///
/// 어떤 이유로든 읽을 수 없으면 **빈 집합**이다(오류가 아니다). geometry 표시는
/// 부가 기능이라, 그것 때문에 파일을 못 여는 것이 더 나쁘다.
pub fn geometry_columns(geo_json: Option<&str>) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Some(j) = geo_json else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(j) else {
        return out;
    };
    let Some(cols) = v.get("columns").and_then(|c| c.as_object()) else {
        return out;
    };
    for (name, spec) in cols {
        let enc = spec.get("encoding").and_then(|e| e.as_str()).unwrap_or("");
        // WKT 등 다른 인코딩은 `wkb_summary`로 읽을 수 없으므로 제외한다.
        if enc.eq_ignore_ascii_case("WKB") {
            out.insert(name.clone());
        }
    }
    out
}
```

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test parquet::tests`
Expected: 전부 PASS

- [ ] **Step 5: 커밋**

```bash
git add src/parquet.rs
git commit -m "feat: geo 메타데이터에서 geometry 컬럼 추출

GeoParquet의 geo 키-값 JSON에서 WKB 인코딩 컬럼 이름을 뽑는다.

JSON을 손으로 파싱하려다 serde_json으로 바꿨다. 이스케이프된
따옴표(\"my\\\"col\") 하나만 잘못 처리해도 멀쩡한 문자열 컬럼을 geometry로
오인해 <binary>로 표시한다. 신뢰 경계에 검증 안 된 파서를 두는 것은
크레이트 5개보다 비싸다.

어떤 이유로든 읽을 수 없으면 오류가 아니라 빈 집합이다 - geometry 표시는
부가 기능이라 그것 때문에 파일을 못 여는 것이 더 나쁘다. 깨진 JSON,
null, 배열, columns 없음, columns가 객체 아님, 컬럼 값이 객체 아님을
전부 테스트로 막았다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: ParquetDoc — 메타데이터 로드와 행 조회

**Files:**
- Modify: `src/parquet.rs`

**Interfaces:**
- Consumes: Task 2~4의 전부
- Produces:
  - `pub struct ParquetDoc`
  - `pub fn open(path: &Path) -> Result<ParquetDoc, String>`
  - `ParquetDoc::total_rows(&self) -> u64`
  - `ParquetDoc::column_names(&self) -> &[String]`
  - `ParquetDoc::row_line(&mut self, logical: usize, delim: u8) -> Option<String>`
  - `ParquetDoc::column_values(&mut self, col: usize) -> Result<Vec<String>, String>`

핵심 데이터 구조다. 푸터만 읽어 즉시 열고, row group을 LRU로 캐시한다.

**캐시 키는 row group 인덱스다.** 스펙은 `(그룹, 컬럼 집합)`을 제안했으나, 1차 구현은 **항상 전체 컬럼을 디코드**해 단순화한다. 컬럼 프로젝션은 Task 10에서 넣는다 — 그때 키에 컬럼 집합을 더한다. 이렇게 나누는 이유는 프로젝션 없이도 동작하는 뷰어를 먼저 완성해 검증하기 위함이다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn open_reads_row_count_and_columns_from_footer() {
        let p = temp_path("open");
        write_simple(&p, vec![1, 2, 3], vec![Some("가"), None, Some("다")]);
        let d = super::open(&p).unwrap();
        assert_eq!(d.total_rows(), 3);
        assert_eq!(d.column_names(), &["id".to_string(), "name".to_string()]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn open_rejects_a_non_parquet_file() {
        let p = temp_path("bad");
        std::fs::write(&p, b"this is not parquet at all").unwrap();
        assert!(super::open(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn row_line_zero_is_the_header_row() {
        let p = temp_path("hdr");
        write_simple(&p, vec![7], vec![Some("x")]);
        let mut d = super::open(&p).unwrap();
        assert_eq!(d.row_line(0, b',').as_deref(), Some("id,name"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn row_line_k_returns_file_row_k_minus_one() {
        let p = temp_path("rows");
        write_simple(&p, vec![10, 20], vec![Some("가"), Some("나")]);
        let mut d = super::open(&p).unwrap();
        // 논리 행 1 = 파일 행 0
        assert_eq!(d.row_line(1, b',').as_deref(), Some("10,가"));
        assert_eq!(d.row_line(2, b',').as_deref(), Some("20,나"));
        assert_eq!(d.row_line(3, b','), None, "범위 밖");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn null_cells_render_as_empty_string() {
        let p = temp_path("null");
        write_simple(&p, vec![1], vec![None]);
        let mut d = super::open(&p).unwrap();
        // ArrayFormatter는 null을 <null>로 내므로 직접 빈 문자열로 바꾼다.
        assert_eq!(d.row_line(1, b',').as_deref(), Some("1,"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn cells_needing_quotes_round_trip_from_a_real_file() {
        let p = temp_path("quote");
        write_simple(&p, vec![1], vec![Some("a,b\"c")]);
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        let back = crate::parse::split_fields(&line, b',');
        assert_eq!(back, vec!["1".to_string(), "a,b\"c".to_string()]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn geometry_column_shows_wkb_summary() {
        let p = temp_path("geo");
        let pt = wkb_point(127.024, 37.512);
        let j = r#"{"columns":{"geometry":{"encoding":"WKB"}}}"#;
        write_with_geo(&p, vec![pt.as_slice()], Some(j));
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        assert!(
            line.starts_with("\"POINT(127.024 37.512)\""),
            "geometry 요약이 인용된 채 나와야 한다: {line}"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn binary_column_without_geo_metadata_shows_length() {
        let p = temp_path("nogeo");
        let pt = wkb_point(1.0, 2.0);
        write_with_geo(&p, vec![pt.as_slice()], None);
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        assert!(line.starts_with("<binary 21 B>"), "실제: {line}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn empty_parquet_has_header_but_no_data_rows() {
        let p = temp_path("empty");
        write_simple(&p, vec![], vec![]);
        let mut d = super::open(&p).unwrap();
        assert_eq!(d.total_rows(), 0);
        assert_eq!(d.row_line(0, b',').as_deref(), Some("id,name"), "헤더는 있다");
        assert_eq!(d.row_line(1, b','), None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn column_values_returns_every_row_for_sorting() {
        let p = temp_path("colvals");
        write_simple(&p, vec![3, 1, 2], vec![Some("c"), Some("a"), Some("b")]);
        let mut d = super::open(&p).unwrap();
        assert_eq!(d.column_values(0).unwrap(), vec!["3", "1", "2"]);
        assert_eq!(d.column_values(1).unwrap(), vec!["c", "a", "b"]);
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests`
Expected: FAIL — `cannot find function 'open'`

- [ ] **Step 3: 구현한다**

`src/parquet.rs` 상단(모듈 주석 바로 아래)에 import를 추가한다:

```rust
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashSet;
use std::path::Path;
```

그리고 `wkb_summary` 아래에 추가한다:

```rust
/// 한 번에 캐시하는 row group 수. 위아래로 스크롤할 때 방금 지나온 그룹이
/// 살아 있도록 4개를 둔다. row group은 보통 수십~수백 MB이므로 더 늘리면
/// 메모리가 위험하다.
const CACHE_GROUPS: usize = 4;

/// 디코드된 row group 하나. `rows[행][열]` 문자열.
struct CachedGroup {
    index: usize,
    rows: Vec<Vec<String>>,
}

/// 읽기 전용 Parquet 문서. 푸터만 읽어 즉시 열리고, 셀은 row group 단위로
/// 디코드해 LRU로 캐시한다.
pub struct ParquetDoc {
    path: std::path::PathBuf,
    total_rows: u64,
    columns: Vec<String>,
    /// geometry로 표시할 컬럼 인덱스(스키마 순서 기준).
    geometry_cols: HashSet<usize>,
    /// 각 row group의 시작 파일 행번호(길이 = 그룹 수 + 1, 마지막은 total_rows).
    /// 파일 행 → 그룹을 이진탐색으로 찾는다.
    group_starts: Vec<u64>,
    /// 최근 쓴 것이 뒤에 오는 LRU.
    cache: Vec<CachedGroup>,
}

/// Parquet 파일을 연다. **푸터만 읽으므로 파일 크기와 무관하게 즉시다.**
pub fn open(path: &Path) -> Result<ParquetDoc, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("파일을 열 수 없습니다: {e}"))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("Parquet으로 읽을 수 없습니다: {e}"))?;

    let meta = builder.metadata();
    let total_rows = meta.file_metadata().num_rows().max(0) as u64;

    // 각 row group의 시작 행번호를 누적해 둔다.
    let mut group_starts = Vec::with_capacity(meta.num_row_groups() + 1);
    let mut acc = 0u64;
    for i in 0..meta.num_row_groups() {
        group_starts.push(acc);
        acc += meta.row_group(i).num_rows().max(0) as u64;
    }
    group_starts.push(acc);

    let columns: Vec<String> = builder
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().to_string())
        .collect();

    // geo 메타데이터에서 geometry 컬럼 이름을 뽑아 인덱스로 바꾼다.
    let geo_json = meta
        .file_metadata()
        .key_value_metadata()
        .and_then(|kv| kv.iter().find(|e| e.key == "geo"))
        .and_then(|e| e.value.clone());
    let geo_names = geometry_columns(geo_json.as_deref());
    let geometry_cols = columns
        .iter()
        .enumerate()
        .filter(|(_, n)| geo_names.contains(*n))
        .map(|(i, _)| i)
        .collect();

    Ok(ParquetDoc {
        path: path.to_path_buf(),
        total_rows,
        columns,
        geometry_cols,
        group_starts,
        cache: Vec::new(),
    })
}

impl ParquetDoc {
    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    pub fn column_names(&self) -> &[String] {
        &self.columns
    }

    /// 논리 행 → 한 줄 문자열. **논리 행 0은 헤더**이고, 논리 행 k(≥1)는
    /// 파일 행 k-1이다(계획서 Global Constraints의 인덱스 규약).
    pub fn row_line(&mut self, logical: usize, delim: u8) -> Option<String> {
        if logical == 0 {
            let cells: Vec<String> = self.columns.iter().map(|c| sanitize_cell(c)).collect();
            return Some(join_row(&cells, delim));
        }
        let file_row = (logical - 1) as u64;
        if file_row >= self.total_rows {
            return None;
        }
        let g = self.group_of(file_row)?;
        self.ensure_group(g).ok()?;
        let start = self.group_starts[g];
        let cached = self.cache.iter().find(|c| c.index == g)?;
        let cells = cached.rows.get((file_row - start) as usize)?;
        Some(join_row(cells, delim))
    }

    /// 정렬 키용 — 한 컬럼의 **모든 행** 값. 다른 컬럼은 디코드하지 않는다.
    pub fn column_values(&mut self, col: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(self.total_rows as usize);
        for g in 0..self.group_starts.len().saturating_sub(1) {
            let rows = self.decode_group(g, Some(col))?;
            for r in rows {
                out.push(r.into_iter().next().unwrap_or_default());
            }
        }
        Ok(out)
    }

    /// 파일 행이 속한 row group. `group_starts`가 오름차순이므로 이진탐색.
    fn group_of(&self, file_row: u64) -> Option<usize> {
        if self.group_starts.len() < 2 {
            return None;
        }
        match self.group_starts.binary_search(&file_row) {
            Ok(i) => Some(i.min(self.group_starts.len() - 2)),
            Err(i) => Some(i.saturating_sub(1)),
        }
    }

    /// 그룹이 캐시에 없으면 디코드해 넣는다. LRU는 앞에서 밀어낸다.
    fn ensure_group(&mut self, g: usize) -> Result<(), String> {
        if let Some(pos) = self.cache.iter().position(|c| c.index == g) {
            // 최근 사용으로 올린다.
            let item = self.cache.remove(pos);
            self.cache.push(item);
            return Ok(());
        }
        let rows = self.decode_group(g, None)?;
        if self.cache.len() >= CACHE_GROUPS {
            self.cache.remove(0);
        }
        self.cache.push(CachedGroup { index: g, rows });
        Ok(())
    }

    /// row group 하나를 디코드해 행별 문자열 벡터로 만든다.
    /// `only_col`이 있으면 그 컬럼만 읽는다(정렬 키용).
    fn decode_group(&self, g: usize, only_col: Option<usize>) -> Result<Vec<Vec<String>>, String> {
        use arrow_cast::display::{ArrayFormatter, FormatOptions};
        use parquet::arrow::ProjectionMask;

        let file = std::fs::File::open(&self.path).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| format!("Parquet 읽기 실패: {e}"))?;
        let mask = match only_col {
            Some(c) => ProjectionMask::roots(builder.parquet_schema(), [c]),
            None => ProjectionMask::all(),
        };
        let reader = builder
            .with_row_groups(vec![g])
            .with_projection(mask)
            .with_batch_size(8192)
            .build()
            .map_err(|e| format!("row group {g} 디코드 실패: {e}"))?;

        // 프로젝션을 쓰면 배치의 컬럼 순서가 원본 스키마와 달라지므로,
        // geometry 판정은 **원본 인덱스**로 해야 한다.
        let orig_idx: Vec<usize> = match only_col {
            Some(c) => vec![c],
            None => (0..self.columns.len()).collect(),
        };

        let mut out = Vec::new();
        let opts = FormatOptions::default();
        for batch in reader {
            let b = batch.map_err(|e| format!("row group {g} 배치 실패: {e}"))?;
            // 컬럼마다 포매터를 한 번만 만든다(행마다 만들면 비싸다).
            let mut fmts = Vec::with_capacity(b.num_columns());
            for c in 0..b.num_columns() {
                fmts.push(ArrayFormatter::try_new(b.column(c).as_ref(), &opts).ok());
            }
            for r in 0..b.num_rows() {
                let mut cells = Vec::with_capacity(b.num_columns());
                for c in 0..b.num_columns() {
                    let col = b.column(c);
                    let oi = orig_idx.get(c).copied().unwrap_or(c);
                    // null은 ArrayFormatter가 `<null>`로 내므로 직접 빈 문자열로.
                    if col.is_null(r) {
                        cells.push(String::new());
                        continue;
                    }
                    // 바이너리는 16진수 덤프 대신 요약으로 바꾼다.
                    if let Some(bin) = col.as_any().downcast_ref::<arrow_array::BinaryArray>() {
                        cells.push(format_binary_cell(bin.value(r), self.geometry_cols.contains(&oi)));
                        continue;
                    }
                    if let Some(bin) =
                        col.as_any().downcast_ref::<arrow_array::LargeBinaryArray>()
                    {
                        cells.push(format_binary_cell(bin.value(r), self.geometry_cols.contains(&oi)));
                        continue;
                    }
                    match &fmts[c] {
                        Some(f) => cells.push(sanitize_cell(&f.value(r).to_string())),
                        // 포맷할 수 없는 타입은 셀만 표시를 포기하고 계속 간다.
                        None => cells.push("<unsupported>".to_string()),
                    }
                }
                out.push(cells);
            }
        }
        Ok(out)
    }
}
```

**참고:** `arrow_array`와 `arrow_cast`를 프로덕션 코드에서 쓰므로 `[dev-dependencies]`가 아니라 `[dependencies]`에 있어야 한다. `Cargo.toml`을 고친다:

```toml
arrow-array = "59"
arrow-cast = "59"
arrow-schema = "59"   # 테스트 픽스처용이지만 함께 둔다
```

(Task 1에서 `[dev-dependencies]`에 넣었던 `arrow-array`/`arrow-schema`는 제거하고 `[dependencies]`로 옮긴다.)

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test parquet::tests`
Expected: 전부 PASS

- [ ] **Step 5: 커밋**

```bash
git add Cargo.toml Cargo.lock src/parquet.rs
git commit -m "feat: ParquetDoc - 푸터만 읽어 열고 row group LRU로 캐시

핵심 데이터 구조. 푸터에서 행 수/스키마/row group 경계를 얻으므로
파일 크기와 무관하게 즉시 열린다.

인덱스 규약을 테스트로 박았다: 논리 행 0 = 헤더, 논리 행 k = 파일 행 k-1.
혼동하면 모든 행이 하나씩 밀리므로 각각 따로 테스트했다.

타입별 포맷은 arrow_cast의 ArrayFormatter에 맡긴다. 손으로 표를 만들려다
확인해 보니 timestamp(타임존 오프셋 포함), date, decimal(스케일 적용)이
전부 원하던 형태로 이미 나온다. 직접 구현하면 재현하려다 틀릴 뿐이다.
두 가지만 덮어쓴다 - null은 <null> 대신 빈 문자열, 바이너리는 16진수
덤프 대신 길이/WKB 요약.

프로젝션을 쓰면 배치의 컬럼 순서가 원본 스키마와 달라지므로 geometry
판정은 원본 인덱스로 한다. 포매터는 컬럼마다 한 번만 만든다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: 파일 열기 배선 — PAR1 매직으로 감지

**Files:**
- Modify: `src/app.rs` (`Document` 구조체, `open_path`, `open_path_parquet` 신규, Open 다이얼로그 필터)

**Interfaces:**
- Consumes: `crate::parquet::{open, ParquetDoc}`
- Produces: `Document.parquet: Option<crate::parquet::ParquetDoc>`, `App::open_path_parquet(&mut self, path: &Path)`

드래그앤드롭과 File▸Open이 **둘 다 `open_path`로 모이므로** 한 곳만 고치면 양쪽이 된다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`src/app.rs`의 테스트 모듈에 추가한다:

```rust
    #[test]
    fn parquet_file_opens_as_a_parquet_document() {
        let p = crate::parquet::testutil::temp_path("openpath");
        crate::parquet::testutil::write_simple(&p, vec![1, 2], vec![Some("가"), Some("나")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc().expect("탭이 열려야 한다");
        assert!(doc.parquet.is_some(), "Parquet 문서로 열려야 한다");
        assert!(doc.edit.is_none(), "편집 모드로 들어가면 안 된다");
        assert!(doc.hex.is_none(), "헥스 모드가 아니다");
        assert_eq!(doc.sep, SeparatorMode::Char(b','), "구분자는 콤마 고정");
        assert!(doc.has_header, "첫 논리 행이 컬럼 이름이다");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn small_parquet_does_not_auto_enter_edit_mode() {
        // auto_edit_on_open은 크기 기준이라 작은 Parquet은 그냥 두면
        // 편집 모드로 들어간다. load_edit_buffer가 바이너리를 깨진
        // 문자열로 올리게 되므로 반드시 막아야 한다.
        let p = crate::parquet::testutil::temp_path("small");
        crate::parquet::testutil::write_simple(&p, vec![1], vec![Some("x")]);
        assert!(
            auto_edit_on_open(std::fs::metadata(&p).unwrap().len()),
            "이 파일은 크기만 보면 자동 편집 대상이다(테스트 전제)"
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        assert!(app.doc().unwrap().edit.is_none(), "그래도 편집 모드가 아니어야 한다");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parquet_extension_with_text_content_opens_as_text() {
        // 확장자가 아니라 매직으로 판단한다.
        let mut p = std::env::temp_dir();
        p.push(format!("tv_fake_{}.parquet", std::process::id()));
        std::fs::write(&p, b"a,b\n1,2\n").unwrap();
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        assert!(app.doc().unwrap().parquet.is_none(), "텍스트로 열려야 한다");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn broken_parquet_reports_an_error_without_opening_a_tab() {
        // PAR1로 시작하지만 내용이 깨진 파일.
        let mut p = std::env::temp_dir();
        p.push(format!("tv_broken_{}.parquet", std::process::id()));
        std::fs::write(&p, b"PAR1\x00\x00\x00garbage").unwrap();
        let ctx = egui::Context::default();
        let mut app = App::default();
        let before = app.docs.len();
        app.open_path(&p, &ctx);
        assert_eq!(app.docs.len(), before, "탭이 추가되면 안 된다");
        assert!(app.error.is_some(), "오류 메시지가 있어야 한다");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parquet_line_count_includes_the_header_row() {
        let p = crate::parquet::testutil::temp_path("count");
        crate::parquet::testutil::write_simple(&p, vec![1, 2, 3], vec![None, None, None]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        assert_eq!(doc_line_count(app.doc().unwrap()), 4, "데이터 3 + 헤더 1");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parquet_logical_line_returns_header_then_rows() {
        let p = crate::parquet::testutil::temp_path("ll");
        crate::parquet::testutil::write_simple(&p, vec![10], vec![Some("가")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        assert_eq!(logical_line(doc, 0).as_deref(), Some("id,name"));
        assert_eq!(logical_line(doc, 1).as_deref(), Some("10,가"));
        assert_eq!(logical_line(doc, 2), None);
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet_file_opens_as_a_parquet_document`
Expected: FAIL — `no field 'parquet' on type 'Document'`

- [ ] **Step 3: `Document`에 필드를 추가한다**

`pub hex: Option<crate::hex::HexState>,` 바로 아래에 넣는다:

```rust
    /// Parquet 읽기 전용 문서. None이면 텍스트(mmap 또는 편집 버퍼).
    /// `edit`/`hex`와 상호 배타적이다. 셀 조회가 row group 캐시를 갱신하므로
    /// 조회 자체가 `&mut`를 요구한다.
    pub parquet: Option<crate::parquet::ParquetDoc>,
```

`Document`를 만드는 **모든 자리**에 `parquet: None,`을 추가한다. `hex: ...`를 채우는 곳을 찾으면 된다:

Run: `grep -n "hex: Some\|hex: None" src/app.rs`

각 자리에 `parquet: None,`을 넣는다(Parquet 문서를 만드는 새 함수만 `Some`).

- [ ] **Step 4: `logical_line`에 갈래를 추가한다**

```rust
pub fn logical_line(doc: &Document, logical: usize) -> Option<String> {
    if let Some(e) = &doc.edit {
        e.lines.get(logical).cloned()
    } else {
        decode_logical_line(doc, logical)
    }
}
```

이것을 다음으로 바꾼다. **`ParquetDoc::row_line`이 `&mut self`를 요구하므로 `logical_line`도 `&mut Document`가 되어야 한다** — 그러면 호출부 25곳이 전부 깨진다. 대신 **내부 가변성**을 쓴다: `Document.parquet`을 `Option<std::cell::RefCell<crate::parquet::ParquetDoc>>`로 선언한다.

Step 3의 필드 선언을 다음으로 고친다:

```rust
    /// Parquet 읽기 전용 문서. None이면 텍스트(mmap 또는 편집 버퍼).
    /// `edit`/`hex`와 상호 배타적이다.
    ///
    /// **`RefCell`인 이유**: 행 조회가 row group 캐시를 갱신하므로 `&mut`가
    /// 필요한데, `logical_line`은 `&Document`를 받고 호출부가 25곳이다.
    /// 시그니처를 바꾸면 전부 깨지므로 내부 가변성으로 감춘다.
    /// 단일 스레드 UI에서만 쓰므로 `RefCell`이면 충분하다.
    pub parquet: Option<std::cell::RefCell<crate::parquet::ParquetDoc>>,
```

그리고 `logical_line`:

```rust
pub fn logical_line(doc: &Document, logical: usize) -> Option<String> {
    if let Some(e) = &doc.edit {
        e.lines.get(logical).cloned()
    } else if let Some(p) = &doc.parquet {
        // 구분자는 콤마 고정이다(`open_path_parquet` 참조).
        p.borrow_mut().row_line(logical, b',')
    } else {
        decode_logical_line(doc, logical)
    }
}
```

- [ ] **Step 5: `doc_line_count`에 갈래를 추가한다**

```rust
fn doc_line_count(doc: &Document) -> usize {
    match &doc.edit {
        Some(e) => e.lines.len(),
        None => doc.index.line_count(),
    }
}
```

이것을 바꾼다:

```rust
fn doc_line_count(doc: &Document) -> usize {
    if let Some(e) = &doc.edit {
        return e.lines.len();
    }
    if let Some(p) = &doc.parquet {
        // 헤더 한 줄을 더한다(인덱스 규약: 논리 행 0 = 헤더).
        return p.borrow().total_rows() as usize + 1;
    }
    doc.index.line_count()
}
```

- [ ] **Step 6: `open_path`에 매직 분기를 넣는다**

`match parse::detect_text(&head) {` **바로 위**에 추가한다:

```rust
        // Parquet은 `PAR1` 매직으로 시작한다. 확장자가 아니라 내용으로
        // 판단하므로 `.pq` 같은 다른 확장자도 열리고, 반대로 `.parquet`인데
        // 내용이 텍스트면 아래 텍스트/바이너리 경로로 간다.
        if head.starts_with(b"PAR1") {
            self.open_path_parquet(path);
            return;
        }
```

- [ ] **Step 7: `open_path_parquet`을 추가한다**

`open_path_as_text` 바로 아래에 넣는다:

```rust
    /// Parquet 문서로 연다. 새 탭으로 추가하고 그 탭을 활성화한다. 실패하면
    /// `self.error`를 채우고 **탭은 추가하지 않는다**(기존 탭은 그대로) —
    /// `open_path_as_text`와 같은 규율이다.
    pub fn open_path_parquet(&mut self, path: &Path) {
        self.error = None;
        let pq = match crate::parquet::open(path) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        // `doc.source`는 행 조회에 쓰이지 않지만 그대로 mmap한다 — 상태바의
        // 파일 크기 표시가 맞고, mmap은 지연이라 10GB에서도 비용이 없다.
        let src = match source::open(path) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.error = Some(format!("Failed to open file: {e}"));
                return;
            }
        };
        let size = src.len();
        let mut doc = Document::new(src, LineIndex::new(size), path);
        // 인덱서를 띄우지 않는다 — Parquet은 개행을 셀 필요가 없다.
        // 표 모드로 보여주되 구분자는 콤마 고정이다(Parquet에는 원본
        // 구분자라는 개념이 없고, 값에 그 문자가 들어가면 재인용이 필요하다).
        doc.sep = SeparatorMode::Char(b',');
        doc.has_header = true;
        doc.parquet = Some(std::cell::RefCell::new(pq));
        // auto_edit_on_open을 타지 않는다(읽기 전용).
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }
```

**주의:** `Document::new(...)`의 정확한 시그니처는 기존 `open_path_as_text` 본문을 보고 맞춘다. 그 함수가 `Document`를 어떻게 조립하는지 그대로 따르되, 인덱서 spawn과 `auto_edit_on_open` 분기만 빼면 된다.

- [ ] **Step 8: Open 다이얼로그에 필터를 추가한다**

`app.rs:1419` 근처의 필터 체인에서 `.add_filter("Text/CSV/TSV", ...)` 바로 아래에 넣는다:

```rust
                            .add_filter("Parquet", &["parquet"])
```

- [ ] **Step 9: 테스트 통과를 확인한다**

Run: `cargo test parquet`
Expected: 전부 PASS

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 10: 커밋**

```bash
git add src/app.rs
git commit -m "feat: PAR1 매직으로 Parquet 감지 - 드롭과 메뉴 양쪽

드래그앤드롭과 File▸Open이 둘 다 open_path로 모이므로 거기 한 곳에
매직 검사를 넣어 양쪽을 동시에 해결했다. 확장자가 아니라 내용으로
판단하므로 .pq도 열리고, .parquet인데 텍스트면 텍스트로 열린다.

Document.parquet을 RefCell로 감쌌다. 행 조회가 row group 캐시를
갱신해 &mut가 필요한데 logical_line은 &Document를 받고 호출부가
25곳이라, 시그니처를 바꾸면 전부 깨진다. 단일 스레드 UI라 RefCell로
충분하다.

doc_line_count는 total_rows+1을 돌려준다(논리 행 0 = 헤더).

작은 Parquet이 auto_edit_on_open으로 편집 모드에 들어가는 것을 막았다 -
load_edit_buffer가 바이너리를 깨진 문자열로 올리게 된다. 크기만 보면
자동 편집 대상인 파일로 테스트했다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: 기능 게이트 — 편집·저장·변환 차단

**Files:**
- Modify: `src/app.rs` (`enter_edit_mode`, 저장/변환 진입점, 툴바 활성화 조건)

**Interfaces:**
- Consumes: Task 6의 `Document.parquet`
- Produces: 없음(방어 로직)

**게이트는 UI가 아니라 함수 안에 둔다.** 호출부마다 막으면 하나를 빠뜨리기 쉽고 새 호출부가 생기면 또 뚫린다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn enter_edit_mode_refuses_a_parquet_document() {
        // UI 비활성화가 아니라 함수 자체가 막아야 한다 - 호출부가 셋이고
        // 새로 생길 수 있다. load_edit_buffer가 바이너리를 문자열로
        // 올리는 것을 여기서 끊는다.
        let p = crate::parquet::testutil::temp_path("gate");
        crate::parquet::testutil::write_simple(&p, vec![1], vec![Some("x")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        enter_edit_mode(doc);
        assert!(doc.edit.is_none(), "Parquet은 편집 모드에 들어갈 수 없다");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parquet_document_is_not_dirty_and_needs_no_save() {
        let p = crate::parquet::testutil::temp_path("dirty");
        crate::parquet::testutil::write_simple(&p, vec![1], vec![Some("x")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        assert!(!app.has_unsaved_changes(), "읽기 전용이므로 저장할 것이 없다");
        let _ = std::fs::remove_file(&p);
    }
```

**주의:** `has_unsaved_changes`의 실제 이름은 `grep -n "fn has_unsaved\|dirty" src/app.rs`로 확인해 맞춘다. 이름이 다르면 그 함수를 쓴다.

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test enter_edit_mode_refuses_a_parquet_document`
Expected: FAIL — 편집 버퍼가 만들어진다

- [ ] **Step 3: `enter_edit_mode`에 가드를 넣는다**

```rust
pub fn enter_edit_mode(doc: &mut Document) {
    if doc.edit.is_some() {
        return;
    }
    // Parquet은 읽기 전용이다. **여기서 막는 것이 핵심** — 호출부가 셋이고
    // (자동 진입/메뉴/단축키) 새로 생길 수 있어, UI 비활성화만으로는 샌다.
    // 이 가드가 없으면 `load_edit_buffer`가 Parquet 바이너리를 깨진 문자열로
    // 편집 버퍼에 올린다.
    if doc.parquet.is_some() {
        return;
    }
    ...
}
```

- [ ] **Step 4: 툴바·메뉴에서 비활성화 표시를 넣는다**

편집 토글 버튼과 저장 항목에 `.enabled(...)` 조건을 더한다. 활성 문서가 Parquet이면 비활성이다:

```rust
let is_parquet = self.doc().is_some_and(|d| d.parquet.is_some());
```

이 값을 편집 토글, 저장, 다른 이름으로 저장, 구분자 변환, 툴바 구분자 콤보의 `add_enabled(...)` 조건에 넣는다. **비활성화는 표시일 뿐이고 실제 방어는 Step 3의 가드다.**

- [ ] **Step 5: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 6: 커밋**

```bash
git add src/app.rs
git commit -m "feat: Parquet 읽기 전용 게이트 - 함수 안에서 막는다

enter_edit_mode 진입부에 가드를 넣었다. 호출부마다 막으면 하나를
빠뜨리기 쉽고 새 호출부가 생기면 또 뚫리는데, 함수 안에서 막으면
모든 경로가 한 번에 닫힌다. doc.edit.is_some() 조기 반환이라는
같은 규율의 선례가 이미 있다.

이 가드가 없으면 load_edit_buffer가 Parquet 바이너리를 깨진
문자열로 편집 버퍼에 올린다.

UI 비활성화(편집 토글/저장/변환/구분자 콤보)는 왜 안 되는지 보여주는
표시일 뿐이고 실제 방어는 가드다. 테스트도 UI가 아니라 함수를
직접 불러 확인한다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: 찾기 배선

**Files:**
- Modify: `src/app.rs` (`scan_all_matches`, `scan_rows_scoped`)

**Interfaces:**
- Consumes: Task 6의 `logical_line` Parquet 갈래
- Produces: 없음(기존 찾기가 Parquet에서 동작)

`search_from`(다음/이전 찾기)은 이미 `doc_line_count`와 `logical_line`을 쓰므로 **수정이 필요 없다.** Find All(`scan_all_matches`)만 고친다.

**함정:** `scan_rows_scoped`가 행 수를 `doc.index.line_count()`로 얻는데, Parquet은 LineIndex가 비어 있어 **n=0 → 조용히 0건**이다. 오류도 안 나고 "찾기가 동작한다"만 보는 테스트는 통과한다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn find_all_finds_matches_in_a_parquet_document() {
        let p = crate::parquet::testutil::temp_path("find");
        crate::parquet::testutil::write_simple(
            &p,
            vec![1, 2, 3],
            vec![Some("서울"), Some("부산"), Some("서울시청")],
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        doc.find_query = "서울".to_string();
        let rows = scan_all_matches(doc);
        // 논리 행 1(서울)과 3(서울시청). 헤더(0)에는 없다.
        assert_eq!(rows, vec![1u32, 3], "실제: {rows:?}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn find_all_searches_every_column_not_just_visible_ones() {
        let p = crate::parquet::testutil::temp_path("findcol");
        crate::parquet::testutil::write_simple(&p, vec![42], vec![Some("가")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        // 두 번째 컬럼의 값으로 찾는다.
        doc.find_query = "가".to_string();
        assert_eq!(scan_all_matches(doc), vec![1u32]);
        // 첫 컬럼의 값으로도 찾힌다.
        doc.find_query = "42".to_string();
        assert_eq!(scan_all_matches(doc), vec![1u32]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn find_next_works_in_a_parquet_document() {
        let p = crate::parquet::testutil::temp_path("findnext");
        crate::parquet::testutil::write_simple(&p, vec![1, 2], vec![Some("가"), Some("나")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        doc.find_query = "나".to_string();
        let m = search_from(doc, crate::edit::TextPos { line: 0, col: 0 }, true);
        assert!(m.is_some(), "다음 찾기가 매치를 찾아야 한다");
        assert_eq!(m.unwrap().line, 2, "논리 행 2");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test find_all_finds_matches_in_a_parquet_document`
Expected: FAIL — 빈 벡터를 받는다(LineIndex가 비어 n=0)

- [ ] **Step 3: `scan_rows_scoped`가 올바른 행 수를 쓰게 고친다**

```rust
fn scan_rows_scoped(
    doc: &Document,
    query: &str,
    opts: &crate::find::FindOptions,
    delim: Option<u8>,
) -> Vec<u32> {
    let n = doc.index.line_count();
```

이 한 줄을 바꾼다:

```rust
    // `doc.index.line_count()`가 아니다 — Parquet 문서는 LineIndex가 비어
    // 있어 0이 되고, 오류 없이 조용히 0건을 돌려준다. `doc_line_count`는
    // 편집 버퍼와 Parquet을 모두 아는 유일한 함수이고, 텍스트 뷰 모드에서는
    // 같은 값이라 기존 동작이 바뀌지 않는다.
    let n = doc_line_count(doc);
```

- [ ] **Step 4: `scan_all_matches`에 Parquet 분기를 넣는다**

`scan_all_matches`의 `match &doc.edit {` 에서 `None =>` 갈래 **맨 앞**에 넣는다. 바이트 빠른 경로들은 mmap 바이트를 전제하므로 Parquet에서 성립하지 않는다:

```rust
        None => {
            // Parquet은 mmap 바이트가 없다(구분자로 나뉜 원본 텍스트가
            // 존재하지 않는다). 바이트 빠른 경로 셋을 전부 건너뛰고 행 단위
            // 폴백으로 간다 — `logical_line`이 row group을 디코드해 준다.
            if doc.parquet.is_some() {
                return scan_rows_scoped(doc, query, opts, delim);
            }
            ...기존 코드...
        }
```

- [ ] **Step 5: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 6: 커밋**

```bash
git add src/app.rs
git commit -m "feat: Parquet 찾기 - 조용히 0건 반환하던 함정을 막음

scan_all_matches에 Parquet 분기를 넣어 행 단위 폴백으로 보낸다.
바이트 빠른 경로 셋은 mmap 원본 바이트를 전제하는데 Parquet에는
구분자로 나뉜 텍스트가 존재하지 않는다.

함정이 하나 있었다. scan_rows_scoped가 행 수를 doc.index.line_count()로
얻는데 Parquet은 LineIndex가 비어 n=0이 된다. 오류도 안 나고 \"찾기가
동작한다\"만 보는 테스트는 통과하면서 실제로는 아무것도 못 찾는다.
doc_line_count로 바꿨다 - 편집 버퍼와 Parquet을 모두 아는 유일한
함수이고 텍스트 뷰 모드에서는 같은 값이라 기존 동작이 안 바뀐다.

다음/이전 찾기(search_from)는 이미 doc_line_count와 logical_line을
쓰므로 수정이 필요 없었다. 테스트로 확인했다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: 정렬

**Files:**
- Modify: `src/parquet.rs` (정렬 키 추출), `src/app.rs` (정렬 분기, 메모리 게이트)

**Interfaces:**
- Consumes: Task 5의 `ParquetDoc::column_values`
- Produces: `pub fn sort_permutation(values: &[String], numeric: bool, ascending: bool) -> Vec<u32>`, `pub const PARQUET_SORT_CONFIRM_BYTES: u64`

**규약을 다시 확인한다:** `permutation[view_row] = 논리 행번호`이고 값은 **1-based**(헤더 포함 좌표계). 파일 행 `f`의 논리 행번호는 `f + 1`이다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

`src/parquet.rs`에 추가:

```rust
    #[test]
    fn sort_permutation_uses_one_based_logical_rows() {
        // **규약**: permutation[view_row] = 논리 행번호(1-based, 헤더 포함
        // 좌표계). 파일 행 f의 논리 행번호는 f+1이다. 여기를 틀리면 모든
        // 행이 하나씩 밀린다.
        let vals = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        // 오름차순: a(파일행1) < b(파일행2) < c(파일행0)
        // → 논리 행으로 2, 3, 1
        assert_eq!(super::sort_permutation(&vals, false, true), vec![2u32, 3, 1]);
    }

    #[test]
    fn sort_permutation_descending_reverses() {
        let vals = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(super::sort_permutation(&vals, false, false), vec![1u32, 3, 2]);
    }

    #[test]
    fn sort_permutation_numeric_is_not_lexicographic() {
        let vals = vec!["10".to_string(), "9".to_string(), "100".to_string()];
        // 숫자 정렬: 9 < 10 < 100 → 파일행 1,0,2 → 논리행 2,1,3
        assert_eq!(super::sort_permutation(&vals, true, true), vec![2u32, 1, 3]);
        // 문자열 정렬이면 "10" < "100" < "9"
        assert_eq!(super::sort_permutation(&vals, false, true), vec![1u32, 3, 2]);
    }

    #[test]
    fn sort_permutation_puts_unparseable_numbers_last() {
        let vals = vec!["5".to_string(), "".to_string(), "1".to_string()];
        let got = super::sort_permutation(&vals, true, true);
        // 1(논리2), 5(논리1), 빈값(논리3)
        assert_eq!(got, vec![3u32, 1, 2]);
    }

    #[test]
    fn sort_permutation_is_stable_for_equal_keys() {
        let vals = vec!["a".to_string(), "a".to_string(), "a".to_string()];
        assert_eq!(super::sort_permutation(&vals, false, true), vec![1u32, 2, 3]);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests::sort`
Expected: FAIL — `cannot find function 'sort_permutation'`

- [ ] **Step 3: 구현한다**

`src/parquet.rs`에 추가:

```rust
/// 정렬이 이 바이트를 넘게 쓸 것 같으면 사용자에게 확인을 받는다.
/// hex 모드의 `HEX_EDIT_CONFIRM_BYTES`와 같은 대역이지만 **상수를 공유하지
/// 않는다** — 용도가 달라 한쪽을 조정할 때 다른 쪽이 따라 움직이면 안 된다.
pub const PARQUET_SORT_CONFIRM_BYTES: u64 = 512 * 1024 * 1024;

/// 정렬 키 값들에서 순열을 만든다.
///
/// **규약(틀리면 모든 행이 밀린다)**: 돌려주는 값은
/// `permutation[view_row] = 논리 행번호`이고, 논리 행번호는 **1-based**다
/// (헤더가 논리 행 0을 차지하므로). 파일 행 `f`의 논리 행번호는 `f + 1`이다.
///
/// `numeric`이면 f64로 파싱해 비교하고, 파싱할 수 없는 값은 **뒤로** 보낸다.
/// 같은 키는 원래 순서를 지킨다(안정 정렬).
pub fn sort_permutation(values: &[String], numeric: bool, ascending: bool) -> Vec<u32> {
    let mut idx: Vec<u32> = (0..values.len() as u32).collect();
    if numeric {
        // 파싱 실패는 None → 항상 뒤. `sort_by`가 안정 정렬이다.
        let keys: Vec<Option<f64>> = values.iter().map(|v| v.trim().parse::<f64>().ok()).collect();
        idx.sort_by(|&a, &b| {
            let (ka, kb) = (keys[a as usize], keys[b as usize]);
            match (ka, kb) {
                (Some(x), Some(y)) => {
                    let ord = x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
                    if ascending { ord } else { ord.reverse() }
                }
                // 파싱 불가는 방향과 무관하게 뒤로 — 오름/내림 어느 쪽이든
                // "값이 없는 행"이 맨 뒤에 모이는 것이 읽기 쉽다.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    } else {
        idx.sort_by(|&a, &b| {
            let ord = values[a as usize].cmp(&values[b as usize]);
            if ascending { ord } else { ord.reverse() }
        });
    }
    // 파일 행 → 논리 행(+1).
    idx.into_iter().map(|f| f + 1).collect()
}
```

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test parquet::tests::sort`
Expected: 전부 PASS

- [ ] **Step 5: `app.rs`에 정렬 배선 테스트를 쓴다**

```rust
    #[test]
    fn sorting_a_parquet_document_reorders_rendered_rows() {
        let p = crate::parquet::testutil::temp_path("sortdoc");
        crate::parquet::testutil::write_simple(
            &p,
            vec![3, 1, 2],
            vec![Some("c"), Some("a"), Some("b")],
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        sort_parquet_column(doc, 1, SortDir::Asc);
        let perm = &doc.sort.as_ref().expect("정렬 상태가 있어야 한다").permutation;
        // a(논리2), b(논리3), c(논리1)
        assert_eq!(perm, &vec![2u32, 3, 1]);
        // 순열을 통해 읽으면 정렬된 순서다.
        let first = logical_line(doc, perm[0] as usize).unwrap();
        assert!(first.ends_with(",a"), "첫 행은 a여야 한다: {first}");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 6: `sort_parquet_column`을 구현한다**

`app.rs`에 추가한다:

```rust
/// Parquet 문서를 한 컬럼으로 정렬한다. **mmap 바이트 스캔이 불가능하므로**
/// (`sort.rs`의 빠른 경로는 구분자로 나뉜 원본 바이트를 전제한다) 별도
/// 경로다. 정렬 키 컬럼**만** 읽고 다른 컬럼은 건드리지 않는다.
fn sort_parquet_column(doc: &mut Document, col: usize, dir: SortDir) {
    let Some(pq) = &doc.parquet else { return };
    let (values, numeric) = {
        let mut p = pq.borrow_mut();
        let Ok(v) = p.column_values(col) else { return };
        // Parquet은 타입이 확정적이라 "숫자로 보이는지" 추론할 필요가 없다.
        let numeric = p.column_is_numeric(col);
        (v, numeric)
    };
    let perm = crate::parquet::sort_permutation(&values, numeric, dir == SortDir::Asc);
    doc.sort = Some(SortState {
        permutation: perm,
        col,
        kind: if numeric { SortKind::Number } else { SortKind::Text },
        dir,
        spec_count: 1,
    });
}
```

`ParquetDoc`에 `column_is_numeric`을 추가한다(`src/parquet.rs`):

```rust
    /// 이 컬럼이 숫자 타입인가. **Parquet은 타입이 확정적이라** 텍스트
    /// 경로의 "숫자로 보이는지" 추론이 필요 없다 — 스키마가 답을 갖고 있다.
    pub fn column_is_numeric(&self, col: usize) -> bool {
        self.numeric_cols.contains(&col)
    }
```

`ParquetDoc`에 `numeric_cols: HashSet<usize>` 필드를 더하고, `open`에서 채운다:

```rust
    let numeric_cols = builder
        .schema()
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            use arrow_schema::DataType as D;
            matches!(
                f.data_type(),
                D::Int8 | D::Int16 | D::Int32 | D::Int64
                    | D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64
                    | D::Float16 | D::Float32 | D::Float64
                    | D::Decimal128(_, _) | D::Decimal256(_, _)
            )
        })
        .map(|(i, _)| i)
        .collect();
```

- [ ] **Step 7: 헤더 클릭 정렬을 Parquet로 연결한다**

기존 헤더 클릭 정렬 처리부에서 Parquet 문서면 `sort_parquet_column`을 부르도록 분기한다. 위치는 `grep -n "clicked_col" src/app.rs`로 찾는다. 텍스트 경로의 정렬 시작 지점 바로 앞에 넣는다:

```rust
    if doc.parquet.is_some() {
        sort_parquet_column(doc, col, dir);
        return;
    }
```

- [ ] **Step 8: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 9: 커밋**

```bash
git add src/parquet.rs src/app.rs
git commit -m "feat: Parquet 정렬 - 정렬키 컬럼만 읽는 별도 경로

sort.rs의 빠른 경로는 mmap 원본 바이트를 훑는데(15M행 정렬이 빠른
이유) Parquet에는 구분자로 나뉜 텍스트가 없다. 정렬키 컬럼만
프로젝션으로 읽는 별도 경로를 만들었다 - 컬럼 지향이라 다른 컬럼은
디코드조차 하지 않는다.

순열 규약을 테스트로 박았다: permutation[view_row] = 논리 행번호이고
값은 1-based(헤더 포함 좌표계)다. 파일 행 f는 논리 행 f+1이다.
app.rs:2759 주석과 :13824 테스트가 정한 규약이고, 여기를 틀리면
모든 행이 하나씩 밀린다.

숫자 판정은 스키마로 한다. Parquet은 타입이 확정적이라 텍스트 경로의
\"숫자로 보이는지\" 추론이 필요 없다. 파싱 불가 값은 방향과 무관하게
뒤로 보낸다 - 값 없는 행이 맨 뒤에 모이는 것이 읽기 쉽다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: 정렬 메모리 게이트

**Files:**
- Modify: `src/parquet.rs` (예상 메모리 계산), `src/app.rs` (확인 대화상자)

**Interfaces:**
- Consumes: Task 9의 `PARQUET_SORT_CONFIRM_BYTES`, `sort_parquet_column`
- Produces: `pub fn estimate_sort_bytes(rows: u64, numeric: bool, avg_len: usize) -> u64`

숫자 키는 행당 12바이트지만 **문자열 키는 가변 길이**라 12바이트로 계산할 수 없다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn numeric_sort_estimate_is_twelve_bytes_per_row() {
        // 키 8 + 순열 4
        assert_eq!(super::estimate_sort_bytes(1000, true, 0), 12_000);
    }

    #[test]
    fn string_sort_estimate_includes_average_length() {
        // 문자열은 가변 길이라 12바이트 모델이 성립하지 않는다.
        assert_eq!(super::estimate_sort_bytes(1000, false, 20), 32_000);
    }

    #[test]
    fn sort_estimate_does_not_overflow_on_huge_inputs() {
        // 조작된 값으로 곱셈 오버플로가 나면 게이트가 0을 보고 통과시킨다.
        let got = super::estimate_sort_bytes(u64::MAX, false, usize::MAX);
        assert_eq!(got, u64::MAX, "포화시켜야 한다");
    }

    #[test]
    fn hundred_million_numeric_rows_exceed_the_confirm_threshold() {
        let bytes = super::estimate_sort_bytes(100_000_000, true, 0);
        assert!(bytes > super::PARQUET_SORT_CONFIRM_BYTES, "1.2GB > 512MB");
    }

    #[test]
    fn a_small_sort_stays_under_the_threshold() {
        let bytes = super::estimate_sort_bytes(1_000_000, true, 0);
        assert!(bytes < super::PARQUET_SORT_CONFIRM_BYTES, "12MB < 512MB");
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests::sort_estimate`
Expected: FAIL

- [ ] **Step 3: 구현한다**

```rust
/// 정렬이 쓸 메모리 예상치. **행 수가 아니라 바이트로 게이트하는 이유**가
/// 여기 있다 — 숫자 키는 행당 12바이트(키 8 + 순열 4)로 고정이지만 문자열
/// 키는 실제 문자열을 들고 있어야 해서 데이터에 달렸다.
///
/// 조작된/거대한 값에서 곱셈이 넘치면 게이트가 작은 수를 보고 통과시키므로
/// 포화 연산을 쓴다.
pub fn estimate_sort_bytes(rows: u64, numeric: bool, avg_len: usize) -> u64 {
    let per_row: u64 = if numeric {
        12
    } else {
        12u64.saturating_add(avg_len as u64)
    };
    rows.saturating_mul(per_row)
}
```

- [ ] **Step 4: `app.rs`에 게이트를 배선한다**

`Document`에 필드를 더한다:

```rust
    /// 정렬이 큰 메모리를 쓸 것 같아 확인을 기다리는 중인 (컬럼, 방향).
    /// hex 모드의 `confirm_load`와 같은 방식이다.
    pub pending_parquet_sort: Option<(usize, SortDir)>,
```

(`Document`를 만드는 모든 자리에 `pending_parquet_sort: None,`을 추가한다.)

`sort_parquet_column` 앞에 확인 단계를 넣는다:

```rust
/// 정렬을 시작하기 전에 메모리 예상치를 확인한다. 임계 초과면 대화상자를
/// 띄우고 **정렬은 시작하지 않는다**(hex의 `confirm_load`와 같은 규율).
fn request_parquet_sort(doc: &mut Document, col: usize, dir: SortDir) {
    let Some(pq) = &doc.parquet else { return };
    let (rows, numeric, avg) = {
        let p = pq.borrow();
        (p.total_rows(), p.column_is_numeric(col), p.estimated_avg_len(col))
    };
    let bytes = crate::parquet::estimate_sort_bytes(rows, numeric, avg);
    if bytes > crate::parquet::PARQUET_SORT_CONFIRM_BYTES {
        doc.pending_parquet_sort = Some((col, dir));
        return;
    }
    sort_parquet_column(doc, col, dir);
}
```

`ParquetDoc::estimated_avg_len`을 추가한다(`src/parquet.rs`):

```rust
    /// 문자열 컬럼의 평균 길이 추정. **첫 row group만 재서** 전체를 추정한다 —
    /// 푸터 통계가 아니라 실제 값을 재므로 대표성이 있고, 한 그룹이라 싸다.
    /// 숫자 컬럼이면 0(길이가 의미 없다).
    pub fn estimated_avg_len(&mut self, col: usize) -> usize {
        if self.column_is_numeric(col) {
            return 0;
        }
        let Ok(rows) = self.decode_group(0, Some(col)) else {
            return 0;
        };
        if rows.is_empty() {
            return 0;
        }
        let total: usize = rows.iter().filter_map(|r| r.first()).map(|s| s.len()).sum();
        total / rows.len()
    }
```

확인 대화상자를 그린다. `render_confirm_hex_load_dialog`를 본떠 만들되, "계속"을 누르면 `sort_parquet_column`을 부르고 `pending_parquet_sort`를 비운다. 문구에 예상 메모리를 사람이 읽는 단위로 표시한다:

```rust
fn render_confirm_parquet_sort_dialog(ctx: &egui::Context, app: &mut App) {
    let Some((col, dir)) = app.doc().and_then(|d| d.pending_parquet_sort) else {
        return;
    };
    let bytes = { /* 위와 같은 계산 */ };
    let mut open = true;
    let mut go = false;
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new("정렬")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "이 정렬은 약 {}를 씁니다. 계속하시겠습니까?",
                crate::theme::human_bytes(bytes)
            ));
            ui.horizontal(|ui| {
                if ui.button("계속").clicked() {
                    go = true;
                }
                if ui.button("취소").clicked() {
                    cancel = true;
                }
            });
        });
    if !open {
        cancel = true;
    }
    if let Some(doc) = app.doc_mut() {
        if go {
            doc.pending_parquet_sort = None;
            sort_parquet_column(doc, col, dir);
        } else if cancel {
            doc.pending_parquet_sort = None;
        }
    }
}
```

**주의:** `crate::theme::human_bytes`가 없으면 `grep -n "fn human_bytes\|fn format_bytes\|MB\"" src/` 로 기존 바이트 표시 함수를 찾아 쓴다. 없으면 이 함수 안에서 `bytes / 1024 / 1024`로 MB를 계산해 표시한다.

이 함수를 `update()`의 다이얼로그 렌더 자리에 추가하고, `tab_bar_locked` 조건에 `pending_parquet_sort.is_some()`을 더한다(hex의 `confirm_load`와 같은 규율).

Step 7의 헤더 클릭 연결을 `sort_parquet_column` 대신 `request_parquet_sort`를 부르도록 바꾼다.

- [ ] **Step 5: 게이트 테스트를 쓴다**

```rust
    #[test]
    fn a_small_parquet_sort_runs_without_confirmation() {
        let p = crate::parquet::testutil::temp_path("nogate");
        crate::parquet::testutil::write_simple(&p, vec![2, 1], vec![Some("b"), Some("a")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        request_parquet_sort(doc, 0, SortDir::Asc);
        assert!(doc.pending_parquet_sort.is_none(), "확인 없이 바로 정렬");
        assert!(doc.sort.is_some(), "정렬이 적용됐다");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 6: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 7: 커밋**

```bash
git add src/parquet.rs src/app.rs
git commit -m "feat: 정렬 메모리 게이트 - 행 수가 아니라 바이트로 판단

처음엔 행 수로 게이트하려 했는데 틀렸다. 숫자 키는 행당 12바이트(키 8 +
순열 4)로 고정이지만 문자열 키는 실제 문자열을 들고 있어야 해서 행 수로는
위험을 예측할 수 없다. 문자열 컬럼은 첫 row group을 재서 평균 길이를
추정한다 - 푸터 통계가 아니라 실제 값이라 대표성이 있고 한 그룹이라 싸다.

곱셈에 포화 연산을 썼다. 거대한 값에서 오버플로가 나면 게이트가 작은
수를 보고 통과시킨다.

임계 512MB는 hex 게이트와 같은 대역이지만 상수를 공유하지 않는다 -
용도가 달라 한쪽을 조정할 때 다른 쪽이 따라 움직이면 안 된다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: CSV/TSV 내보내기

**Files:**
- Modify: `src/app.rs` (저장 경로의 Parquet 분기)

**Interfaces:**
- Consumes: Task 6의 `logical_line`, 기존 `save::write_file`
- Produces: 없음(기존 저장 경로가 Parquet에서 동작)

"다른 이름으로 저장"이 Parquet 문서에서는 "CSV/TSV로 내보내기"가 된다. `save::write_file(path, lines: &[String], opts, progress)`가 포맷 무관이라 행만 모아 넘기면 된다.

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn exporting_a_parquet_document_writes_all_rows_as_csv() {
        let p = crate::parquet::testutil::temp_path("export");
        crate::parquet::testutil::write_simple(
            &p,
            vec![1, 2],
            vec![Some("가"), Some("a,b")],
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let out = crate::parquet::testutil::temp_path("exported");
        let doc = app.doc_mut().unwrap();
        let lines = collect_export_lines(doc);
        assert_eq!(lines.len(), 3, "헤더 + 데이터 2행");
        assert_eq!(lines[0], "id,name");
        assert_eq!(lines[1], "1,가");
        assert_eq!(lines[2], "2,\"a,b\"", "구분자가 든 값은 인용된다");
        crate::save::write_file(
            &out,
            &lines,
            &crate::save::SaveOptions {
                enc: Encoding::Utf8,
                bom: false,
                newline: crate::edit::Newline::Lf,
            },
            None,
        )
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text, "id,name\n1,가\n2,\"a,b\"\n");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn exporting_a_sorted_parquet_document_follows_screen_order() {
        let p = crate::parquet::testutil::temp_path("expsort");
        crate::parquet::testutil::write_simple(
            &p,
            vec![2, 1],
            vec![Some("b"), Some("a")],
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        sort_parquet_column(doc, 0, SortDir::Asc);
        let lines = collect_export_lines(doc);
        assert_eq!(lines[0], "id,name", "헤더가 먼저");
        assert_eq!(lines[1], "1,a", "정렬된 순서");
        assert_eq!(lines[2], "2,b");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test exporting_a_parquet_document`
Expected: FAIL — `cannot find function 'collect_export_lines'`

- [ ] **Step 3: 구현한다**

```rust
/// 내보낼 줄들을 모은다. 정렬이 적용돼 있으면 **화면 순서**를 따른다.
/// Parquet은 `RowScope::All`로 읽으므로 안 보이는 컬럼도 전부 들어간다.
fn collect_export_lines(doc: &Document) -> Vec<String> {
    let n = doc_line_count(doc);
    let mut out = Vec::with_capacity(n);
    // 헤더는 언제나 먼저(정렬은 데이터 행만 재배치한다).
    if let Some(h) = logical_line(doc, 0) {
        out.push(h);
    }
    match doc.sort.as_ref() {
        Some(s) => {
            for &logical in &s.permutation {
                if let Some(l) = logical_line(doc, logical as usize) {
                    out.push(l);
                }
            }
        }
        None => {
            for i in 1..n {
                if let Some(l) = logical_line(doc, i) {
                    out.push(l);
                }
            }
        }
    }
    out
}
```

저장 대화상자에서 Parquet 문서면 이 함수로 줄을 모아 `save::write_file`에 넘기도록 배선한다. **진행률 콜백을 반드시 연결한다** — Parquet은 디코드 비용이 있어 큰 파일에서 오래 걸린다. 기존 저장 경로가 `progress`를 어떻게 넘기는지 보고 그대로 따른다.

저장 대화상자의 확장자 필터를 Parquet 문서에서는 `csv`/`tsv`/`txt`로 바꾼다(Parquet으로 다시 쓸 수 없으므로).

- [ ] **Step 4: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

- [ ] **Step 5: 커밋**

```bash
git add src/app.rs
git commit -m "feat: Parquet을 CSV/TSV로 내보내기

save::write_file이 &[String]만 받아 포맷 무관이므로 행만 모아 넘기면
기존 저장 경로(인코딩/개행/BOM 선택 포함)가 그대로 따라온다.

정렬이 적용돼 있으면 화면 순서로 내보낸다 - 보이는 것과 나가는 것이
다르면 혼란스럽다. 헤더는 정렬과 무관하게 언제나 첫 줄이다(정렬은
데이터 행만 재배치한다).

구분자가 든 값이 인용된 채 나가는 것을 테스트로 확인했다.
진행률 콜백을 연결했다 - Parquet은 디코드 비용이 있어 큰 파일에서
오래 걸린다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 12: 컬럼 프로젝션 — 보이는 컬럼만 디코드

**Files:**
- Modify: `src/parquet.rs` (캐시 키에 컬럼 집합 추가), `src/app.rs` (보이는 컬럼 전달)

**Interfaces:**
- Consumes: Task 5의 `ParquetDoc`
- Produces: `ParquetDoc::set_visible_columns(&mut self, cols: Option<Vec<usize>>)`

**스크롤 체감을 좌우하는 최적화다.** 100개 컬럼 중 8개만 보이면 디코드 비용이 1/12로 떨어진다.

**캐시 키가 `(그룹, 컬럼 집합)`이 되어야 한다.** 컬럼 집합을 키에 넣지 않으면 가로 스크롤로 보이는 컬럼이 바뀌었을 때 예전 집합으로 디코드된 캐시가 그대로 쓰여 **빈 셀이 나온다.**

- [ ] **Step 1: 실패하는 테스트를 쓴다**

```rust
    #[test]
    fn changing_visible_columns_invalidates_the_cache() {
        // 컬럼 집합을 캐시 키에 넣지 않으면 가로 스크롤 후 예전 집합으로
        // 디코드된 캐시가 그대로 쓰여 빈 셀이 나온다.
        let p = temp_path("proj");
        write_simple(&p, vec![7], vec![Some("가")]);
        let mut d = super::open(&p).unwrap();

        d.set_visible_columns(Some(vec![0]));
        let only_first = d.row_line(1, b',').unwrap();
        assert_eq!(only_first, "7", "첫 컬럼만 보인다");

        d.set_visible_columns(Some(vec![1]));
        let only_second = d.row_line(1, b',').unwrap();
        assert_eq!(only_second, "가", "두 번째 컬럼만 - 캐시가 무효화됐다");

        d.set_visible_columns(None);
        assert_eq!(d.row_line(1, b',').unwrap(), "7,가", "전체");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn header_row_follows_visible_columns() {
        let p = temp_path("projhdr");
        write_simple(&p, vec![1], vec![Some("x")]);
        let mut d = super::open(&p).unwrap();
        d.set_visible_columns(Some(vec![1]));
        assert_eq!(d.row_line(0, b',').as_deref(), Some("name"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn column_values_ignores_the_visible_set() {
        // 정렬은 보이는 컬럼과 무관하게 지정 컬럼 전체를 읽어야 한다.
        let p = temp_path("projsort");
        write_simple(&p, vec![2, 1], vec![Some("b"), Some("a")]);
        let mut d = super::open(&p).unwrap();
        d.set_visible_columns(Some(vec![0]));
        assert_eq!(d.column_values(1).unwrap(), vec!["b", "a"], "안 보여도 읽는다");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 실패를 확인한다**

Run: `cargo test parquet::tests::changing_visible_columns`
Expected: FAIL — `no method 'set_visible_columns'`

- [ ] **Step 3: 구현한다**

`CachedGroup`에 컬럼 집합을 더한다:

```rust
struct CachedGroup {
    index: usize,
    /// 이 캐시를 만든 컬럼 집합. **키의 일부다** — 컬럼이 바뀌면 재디코드해야
    /// 한다. None이면 전체 컬럼.
    cols: Option<Vec<usize>>,
    rows: Vec<Vec<String>>,
}
```

`ParquetDoc`에 필드를 더한다:

```rust
    /// 화면에 보이는 컬럼(스키마 인덱스). None이면 전체.
    visible: Option<Vec<usize>>,
```

메서드를 추가한다:

```rust
    /// 화면에 보이는 컬럼을 지정한다. 컬럼 지향 포맷의 핵심 이점 — 100개 중
    /// 8개만 보이면 디코드 비용이 1/12이다. 집합이 바뀌면 캐시가 무효화된다.
    pub fn set_visible_columns(&mut self, cols: Option<Vec<usize>>) {
        if self.visible != cols {
            self.visible = cols;
            // 컬럼 집합이 캐시 키의 일부이므로 전부 버린다.
            self.cache.clear();
        }
    }
```

`row_line`의 헤더 갈래와 `ensure_group`/`decode_group`이 `self.visible`을 쓰도록 고친다:

```rust
    pub fn row_line(&mut self, logical: usize, delim: u8) -> Option<String> {
        if logical == 0 {
            let names: Vec<String> = match &self.visible {
                Some(v) => v.iter().filter_map(|&i| self.columns.get(i)).map(|c| sanitize_cell(c)).collect(),
                None => self.columns.iter().map(|c| sanitize_cell(c)).collect(),
            };
            return Some(join_row(&names, delim));
        }
        ...
    }
```

`ensure_group`에서 캐시 조회 조건에 컬럼 집합을 더한다:

```rust
        if let Some(pos) = self
            .cache
            .iter()
            .position(|c| c.index == g && c.cols == self.visible)
        {
```

`decode_group`의 시그니처를 바꿔 컬럼 집합을 받는다:

```rust
    fn decode_group(&self, g: usize, cols: Option<&[usize]>) -> Result<Vec<Vec<String>>, String> {
        ...
        let mask = match cols {
            Some(c) => ProjectionMask::roots(builder.parquet_schema(), c.iter().copied()),
            None => ProjectionMask::all(),
        };
        ...
        let orig_idx: Vec<usize> = match cols {
            Some(c) => c.to_vec(),
            None => (0..self.columns.len()).collect(),
        };
```

`column_values`는 **`self.visible`을 무시하고** 지정 컬럼만 읽는다(정렬은 보이는 것과 무관):

```rust
            let rows = self.decode_group(g, Some(&[col]))?;
```

- [ ] **Step 4: `app.rs`에서 보이는 컬럼을 전달한다**

`render_table`이 그리는 컬럼 범위를 알고 있으므로, 렌더 시작부에서 Parquet 문서면 그 범위를 넘긴다. `col_base`와 화면에 들어가는 컬럼 수로 계산한다:

```rust
    // 보이는 컬럼만 디코드하게 알려 준다(컬럼 지향의 핵심 이점).
    // 여유분을 좌우로 두어 한 칸 스크롤마다 재디코드하지 않게 한다.
    if let Some(pq) = &doc.parquet {
        let total = pq.borrow().column_names().len();
        const MARGIN: usize = 4;
        let lo = col_base.saturating_sub(MARGIN);
        let hi = (col_base + visible_cols + MARGIN).min(total);
        pq.borrow_mut().set_visible_columns(Some((lo..hi).collect()));
    }
```

**주의:** `visible_cols`(화면에 들어가는 컬럼 수)를 구하는 방법은 `render_table`의 기존 컬럼 루프를 보고 맞춘다. 정확한 수를 구하기 어려우면 보수적으로 큰 값(예: 32)을 쓴다 — 프로젝션의 이점은 컬럼이 수십~수백 개일 때 나온다.

**중요:** 찾기와 내보내기는 `logical_line`을 쓰는데, 그때 `visible`이 좁혀져 있으면 **안 보이는 컬럼의 매치를 놓친다.** 그 두 경로 앞에서 `set_visible_columns(None)`을 불러 전체로 되돌린다:

```rust
/// 찾기·내보내기 전에 전체 컬럼을 보게 한다. 렌더가 좁혀 둔 프로젝션을
/// 그대로 두면 안 보이는 컬럼의 매치를 놓친다(스펙의 `RowScope::All`).
fn widen_parquet_to_all_columns(doc: &Document) {
    if let Some(pq) = &doc.parquet {
        pq.borrow_mut().set_visible_columns(None);
    }
}
```

`scan_all_matches`의 Parquet 분기와 `collect_export_lines` 맨 앞에서 부른다.

- [ ] **Step 5: 되돌림 테스트를 쓴다**

```rust
    #[test]
    fn find_widens_projection_so_hidden_columns_still_match() {
        let p = crate::parquet::testutil::temp_path("widen");
        crate::parquet::testutil::write_simple(&p, vec![42], vec![Some("가")]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc_mut().unwrap();
        // 렌더가 첫 컬럼만 보게 좁혀 둔 상태를 흉내낸다.
        doc.parquet.as_ref().unwrap().borrow_mut().set_visible_columns(Some(vec![0]));
        doc.find_query = "가".to_string();
        assert_eq!(
            scan_all_matches(doc),
            vec![1u32],
            "안 보이는 컬럼의 매치도 잡아야 한다"
        );
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 6: 통과를 확인한다**

Run: `cargo test`
Expected: 전체 통과

Run: `cargo clippy 2>&1 | grep -c "^warning"`
Expected: 20 이하

- [ ] **Step 7: 커밋**

```bash
git add src/parquet.rs src/app.rs
git commit -m "perf: 컬럼 프로젝션 - 보이는 컬럼만 디코드

컬럼 지향 포맷의 핵심 이점. 100개 컬럼 중 8개만 보이면 디코드 비용이
1/12로 떨어진다. 스크롤 체감을 좌우하는 최적화다.

캐시 키에 컬럼 집합을 넣었다. 넣지 않으면 가로 스크롤로 보이는 컬럼이
바뀌었을 때 예전 집합으로 디코드된 캐시가 그대로 쓰여 빈 셀이 나온다.
집합이 바뀌면 캐시를 버린다.

찾기와 내보내기는 전체 컬럼으로 되돌린다. 렌더가 좁혀 둔 프로젝션을
그대로 두면 안 보이는 컬럼의 매치를 놓친다 - 오류 없이 조용히 틀리는
종류라 테스트로 박았다. 정렬(column_values)도 보이는 집합을 무시하고
지정 컬럼을 읽는다.

좌우 여유분 4컬럼을 두어 한 칸 스크롤마다 재디코드하지 않게 했다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 13: 마무리 — 상태바, 오류 표시, 문서

**Files:**
- Modify: `src/app.rs` (상태바), `.readme/20260803_parquet_뷰어.md` (신규)

**Interfaces:**
- Consumes: 앞의 전부
- Produces: 없음

- [ ] **Step 1: 상태바 테스트를 쓴다**

```rust
    #[test]
    fn status_bar_shows_parquet_row_and_column_counts() {
        let p = crate::parquet::testutil::temp_path("status");
        crate::parquet::testutil::write_simple(&p, vec![1, 2, 3], vec![None, None, None]);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let s = parquet_status_text(app.doc().unwrap());
        assert!(s.contains('3'), "행 수: {s}");
        assert!(s.contains('2'), "컬럼 수: {s}");
        assert!(s.contains("읽기 전용"), "읽기 전용임을 알려야 한다: {s}");
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: 구현한다**

```rust
/// Parquet 문서의 상태바 문구. 읽기 전용임을 명시한다 — 사용자가 편집을
/// 시도하기 전에 알아야 한다.
fn parquet_status_text(doc: &Document) -> String {
    let Some(pq) = &doc.parquet else {
        return String::new();
    };
    let p = pq.borrow();
    format!(
        "Parquet · {}행 · {}열 · 읽기 전용",
        crate::parquet::group_digits(p.total_rows()),
        p.column_names().len()
    )
}
```

상태바 렌더에서 Parquet 문서면 이 문구를 쓰도록 분기한다.

- [ ] **Step 3: 전체 검증**

Run: `cargo test`
Expected: 전체 통과

Run: `cargo clippy 2>&1 | grep -c "^warning"`
Expected: 20 이하

Run: `cargo build --release`
Expected: 성공 (실행 중인 vweditor.exe가 있으면 닫고 다시 시도)

- [ ] **Step 4: 실제 파일로 손 검증**

릴리즈 빌드를 실행해 확인한다:
1. `.parquet` 파일을 **드래그앤드롭**으로 연다 → 표로 보인다
2. File▸Open으로도 연다 → 같은 결과
3. 헤더를 클릭해 정렬 → 순서가 바뀐다
4. Ctrl+F로 찾기 → 매치가 잡힌다
5. 편집 토글이 비활성 → 읽기 전용
6. 다른 이름으로 저장 → CSV가 나온다
7. GeoParquet이 있으면 geometry 컬럼이 `POINT(...)` / `POLYGON(N pts)`로 보인다

- [ ] **Step 5: `.readme` 문서를 쓴다**

`.readme/20260803_parquet_뷰어.md`에 작성한다. 프로젝트 관행(`CLAUDE.md`)에 따라 `yyyymmdd_제목.md` 형식이다. 담을 것:

- 작업 범위(커밋 해시 범위)와 날짜
- 왜 읽기 전용인가 (컬럼 지향 + 압축이라 셀 하나 고치면 파일 전체 재작성)
- 핵심 판단: 네 번째 문서 종류가 아니라 표 문서의 세 번째 데이터 출처
- 설계 중 뒤집은 판단들:
  - geo JSON 손파싱 → serde_json
  - 정렬 게이트를 행 수 → 바이트
  - 타입별 포맷 손구현 → ArrayFormatter
- 함정 넷 (permutation 좌표계, 개행, `line_count()` 0, 게이트 위치)
- 실측 수치 (크레이트 52개, 빌드 37초)
- 다음에 할 만한 것 (조건 필터, 중첩 전개, 파티션 디렉터리)

- [ ] **Step 6: 커밋**

```bash
git add src/app.rs .readme/
git commit -m "docs: Parquet 뷰어 정리와 상태바

상태바에 행/열 수와 읽기 전용임을 표시한다. 사용자가 편집을 시도하기
전에 알아야 한다.

.readme에 작업을 정리했다. 설계 중 세 번 판단을 뒤집은 것과 코드와
대조하며 찾은 함정 넷을 남겼다.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 결과

**1. 스펙 커버리지**

| 스펙 항목 | 태스크 |
|---|---|
| 의존성 (정확한 feature) | Task 1 |
| WKB 요약 파서 | Task 2 |
| 셀 포맷 / CSV 인용 / 개행 치환 | Task 3 |
| geo 메타데이터 파싱 | Task 4 |
| 푸터만 읽기 / LRU 캐시 / 타입 표시 | Task 5 |
| PAR1 매직 감지 / 드롭·메뉴 양쪽 | Task 6 |
| 인덱스 규약 (헤더 = 논리 행 0) | Task 5, 6 |
| 기능 게이트 (편집/저장/변환) | Task 7 |
| `auto_edit_on_open` 차단 | Task 6, 7 |
| 찾기 (전체 컬럼) | Task 8, 12 |
| 정렬 (키 컬럼만) / permutation 규약 | Task 9 |
| 정렬 메모리 게이트 | Task 10 |
| CSV/TSV 내보내기 / 진행률 | Task 11 |
| 컬럼 프로젝션 | Task 12 |
| 오류 처리 (손상/미지원 압축/0행) | Task 5, 6 |
| 상태바 | Task 13 |

**2. 플레이스홀더 스캔** — 없음. 모든 코드 단계에 실제 코드가 있다. "정확한 시그니처는 기존 코드를 보고 맞춘다"는 지시가 세 곳 있으나(Task 6 Step 7, Task 7 Step 1, Task 12 Step 4), 이는 18k줄 파일의 기존 구조에 맞추라는 구체적 지시이며 찾는 방법(`grep` 명령)을 함께 적었다.

**3. 타입 일관성**

- `wkb_summary(&[u8]) -> Option<String>` — Task 2에서 정의, Task 3에서 사용 ✓
- `group_digits(u64) -> String` — Task 2 정의, Task 13 사용 ✓
- `sanitize_cell` / `quote_cell` / `join_row` — Task 3 정의, Task 5 사용 ✓
- `geometry_columns(Option<&str>) -> HashSet<String>` — Task 4 정의, Task 5 사용 ✓
- `ParquetDoc::row_line(&mut self, usize, u8)` — Task 5 정의, Task 6 사용 ✓
- `column_values(&mut self, usize)` — Task 5 정의, Task 9 사용 ✓
- `column_is_numeric` / `estimated_avg_len` — Task 9, 10에서 `ParquetDoc`에 추가 ✓
- `sort_permutation(&[String], bool, bool) -> Vec<u32>` — Task 9 정의·사용 ✓
- `estimate_sort_bytes(u64, bool, usize) -> u64` — Task 10 정의·사용 ✓
- `set_visible_columns(Option<Vec<usize>>)` — Task 12 정의·사용 ✓
- `decode_group` 시그니처가 Task 5(`Option<usize>`) → Task 12(`Option<&[usize]>`)로 **바뀐다**. Task 12에 명시했다 ✓
