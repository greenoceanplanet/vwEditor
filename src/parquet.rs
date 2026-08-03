//! Parquet 읽기 전용 뷰어의 순수 로직 — 셀 포맷, WKB 요약, 행 조립, 캐시.
//! egui 없음. `hex.rs`/`find.rs`/`convert.rs`와 같은 규율이다.
//!
//! **왜 읽기 전용인가.** Parquet은 컬럼별로 흩어진 압축·인코딩 포맷이라
//! 셀 하나를 고치려면 페이지를 풀어 다시 인코딩·압축해야 하고, 크기가
//! 달라져 뒤따르는 모든 청크의 오프셋이 밀리며 푸터 메타데이터를 전부
//! 재계산해야 한다. 사실상 파일 전체 재작성이다. 텍스트 파일에서
//! "수정분만 반영해 저장"이 가능한 것은 바이트 오프셋이 안정적이기
//! 때문인데, Parquet에는 그 성질이 없다.

use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashSet;
use std::path::Path;

/// 한 번에 캐시하는 row group 수. 위아래로 스크롤할 때 방금 지나온 그룹이
/// 살아 있도록 넷을 둔다. row group은 보통 수십~수백 MB이므로 더 늘리면
/// 메모리가 위험하다.
const CACHE_GROUPS: usize = 4;

/// 배치 크기. 한 번에 이만큼씩 디코드해 문자열로 만든다.
const BATCH_ROWS: usize = 8192;

/// 천 단위 쉼표를 넣는다. 셀 값은 인용 규칙을 타므로(`quote_cell`) 쉼표가
/// 컬럼을 깨지 않는다.
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
/// ```text
/// 바이트 0    : 엔디안 (0 = big, 1 = little)
/// 바이트 1..5 : geometry 타입 (u32)
/// 이후        : 타입별 페이로드
/// ```
///
/// 깨진 입력·짧은 입력·모르는 타입은 **None**이다. 호출부가 `<binary N B>`로
/// 폴백한다 — 뷰어가 데이터 문제로 죽으면 안 된다.
///
/// Z/M 차원(타입 코드에 1000/2000/3000을 더해 표현)은 기본 타입으로 환원한다.
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
        Some(if little {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };
    let f64_at = |o: usize| -> Option<f64> {
        let s: [u8; 8] = b.get(o..o + 8)?.try_into().ok()?;
        Some(if little {
            f64::from_le_bytes(s)
        } else {
            f64::from_be_bytes(s)
        })
    };

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
                // 좌표 하나 = f64 두 개 = 16바이트.
                //
                // **버퍼 길이로 검증한다.** checked 연산만으로는 부족하다 —
                // 64비트에서 `u32::MAX * 16`은 usize를 넘지 않아 산술은
                // 멀쩡히 성공하고, 파일에 있을 수 없는 좌표 수를 그대로
                // 보고하게 된다(조작된 값으로 실제 재현). 다음 링의 시작이
                // 버퍼 안이어야만 계속 간다.
                off = off
                    .checked_add(4)?
                    .checked_add((n as usize).checked_mul(16)?)?;
                if off > b.len() {
                    return None;
                }
            }
            Some(format!("POLYGON({} pts)", group_digits(total)))
        }
        4 => Some(format!("MULTIPOINT({} pts)", group_digits(u32_at(5)? as u64))),
        5 => Some(format!(
            "MULTILINESTRING({} parts)",
            group_digits(u32_at(5)? as u64)
        )),
        6 => Some(format!(
            "MULTIPOLYGON({} parts)",
            group_digits(u32_at(5)? as u64)
        )),
        7 => Some(format!(
            "GEOMETRYCOLLECTION({} parts)",
            group_digits(u32_at(5)? as u64)
        )),
        _ => None,
    }
}

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
                // CRLF는 공백 하나로 접는다(둘로 만들면 폭이 달라진다).
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
/// 형태여야 한다 — **이 왕복이 표의 컬럼 정렬을 지탱한다**(`join_row` 참조).
pub fn quote_cell(s: &str, delim: u8) -> String {
    let needs = s
        .bytes()
        .any(|b| b == delim || b == b'"' || b == b'\r' || b == b'\n');
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
///
/// **계약**: `split_fields(&join_row(&cells, d), d) == cells`. 이게 깨지면
/// 표의 모든 컬럼이 어긋난다(테스트 `join_row_round_trips_through_split_fields`).
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

/// `geo` 키-값 메타데이터에서 **WKB로 인코딩된 geometry 컬럼 이름**을 뽑는다.
///
/// GeoParquet은 평범한 Parquet이고 `geo` 키에 JSON이 들어 있다:
/// `{"version":"1.0.0","primary_column":"geometry",
///   "columns":{"geometry":{"encoding":"WKB",...}}}`
///
/// **JSON을 손으로 파싱하지 않는다.** 이스케이프된 따옴표(`"my\"col"`) 하나만
/// 잘못 처리해도 멀쩡한 문자열 컬럼을 geometry로 오인해 `<binary>`로 표시한다.
/// 신뢰 경계에 검증 안 된 파서를 두는 것이 크레이트 몇 개보다 비싸다.
///
/// 어떤 이유로든 읽을 수 없으면 **빈 집합**이다(오류가 아니다). geometry 표시는
/// 부가 기능이라, 그것 때문에 파일을 못 여는 것이 더 나쁘다.
pub fn geometry_columns(geo_json: Option<&str>) -> HashSet<String> {
    let mut out = HashSet::new();
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

/// 정렬이 이 바이트를 넘게 쓸 것 같으면 사용자에게 확인을 받는다.
/// hex 모드의 `HEX_EDIT_CONFIRM_BYTES`와 같은 대역(512MB)이지만 **상수를
/// 공유하지 않는다** — 용도가 달라 한쪽을 조정할 때 다른 쪽이 따라 움직이면
/// 안 된다.
pub const PARQUET_SORT_CONFIRM_BYTES: u64 = 512 * 1024 * 1024;

/// 정렬이 쓸 메모리 예상치.
///
/// **행 수가 아니라 바이트로 게이트하는 이유가 여기 있다.** 숫자 키는 행당
/// 12바이트(키 8 + 순열 4)로 고정이지만, 문자열 키는 실제 문자열을 들고 있어야
/// 해서 데이터에 달렸다 — 행 수만으로는 위험을 예측할 수 없다.
///
/// 거대한/조작된 값에서 곱셈이 넘치면 게이트가 작은 수를 보고 통과시키므로
/// 포화 연산을 쓴다.
pub fn estimate_sort_bytes(rows: u64, numeric: bool, avg_len: usize) -> u64 {
    let per_row: u64 = if numeric {
        12
    } else {
        12u64.saturating_add(avg_len as u64)
    };
    rows.saturating_mul(per_row)
}

/// 정렬 키 값들에서 순열을 만든다.
///
/// **규약(틀리면 모든 행이 밀린다)**: 돌려주는 값은
/// `permutation[view_row] = 논리 행번호`이고, 논리 행번호는 **1-based**다
/// (헤더가 논리 행 0을 차지하므로). 파일 행 `f`의 논리 행번호는 `f + 1`이다.
/// `app.rs:2759` 주석과 그 테스트가 정한 규약이다.
///
/// `numeric`이면 f64로 파싱해 비교하고, **파싱할 수 없는 값은 방향과 무관하게
/// 뒤로** 보낸다(값 없는 행이 맨 뒤에 모이는 것이 읽기 쉽다).
/// 같은 키는 원래 순서를 지킨다(안정 정렬).
pub fn sort_permutation(values: &[String], numeric: bool, ascending: bool) -> Vec<u32> {
    let mut idx: Vec<u32> = (0..values.len() as u32).collect();
    if numeric {
        let keys: Vec<Option<f64>> = values.iter().map(|v| v.trim().parse::<f64>().ok()).collect();
        idx.sort_by(|&a, &b| {
            match (keys[a as usize], keys[b as usize]) {
                (Some(x), Some(y)) => {
                    let ord = x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
                    if ascending {
                        ord
                    } else {
                        ord.reverse()
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    } else {
        idx.sort_by(|&a, &b| {
            let ord = values[a as usize].cmp(&values[b as usize]);
            if ascending {
                ord
            } else {
                ord.reverse()
            }
        });
    }
    // 파일 행 → 논리 행(헤더가 0을 차지하므로 +1).
    idx.into_iter().map(|f| f + 1).collect()
}

/// 디코드된 row group 하나.
struct CachedGroup {
    index: usize,
    /// 이 캐시를 만든 컬럼 집합. **키의 일부다** — 넣지 않으면 가로 스크롤로
    /// 보이는 컬럼이 바뀌었을 때 예전 집합으로 디코드된 캐시가 그대로 쓰여
    /// 빈 셀이 나온다. None이면 전체 컬럼.
    cols: Option<Vec<usize>>,
    /// `rows[행][열]` 문자열.
    rows: Vec<Vec<String>>,
}

/// 읽기 전용 Parquet 문서. **푸터만 읽어 즉시 열리고**, 셀은 row group 단위로
/// 디코드해 LRU로 캐시한다.
pub struct ParquetDoc {
    path: std::path::PathBuf,
    total_rows: u64,
    columns: Vec<String>,
    /// geometry로 표시할 컬럼(스키마 순서 기준 인덱스).
    geometry_cols: HashSet<usize>,
    /// 숫자 타입 컬럼. 정렬 키 판정에 쓴다 — Parquet은 타입이 확정적이라
    /// 텍스트 경로처럼 "숫자로 보이는지" 추론할 필요가 없다.
    numeric_cols: HashSet<usize>,
    /// 각 row group의 시작 파일 행번호. 길이는 그룹 수 + 1이고 마지막은
    /// `total_rows`다. 파일 행 → 그룹을 이진탐색으로 찾는다.
    group_starts: Vec<u64>,
    /// 화면에 보이는 컬럼(스키마 인덱스). None이면 전체.
    visible: Option<Vec<usize>>,
    /// 최근 쓴 것이 뒤에 오는 LRU.
    cache: Vec<CachedGroup>,
}

/// Parquet 파일을 연다. **푸터만 읽으므로 파일 크기와 무관하게 즉시다** —
/// CSV처럼 개행을 세러 전체를 훑지 않는다.
pub fn open(path: &Path) -> Result<ParquetDoc, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("파일을 열 수 없습니다: {e}"))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("Parquet으로 읽을 수 없습니다: {e}"))?;

    let meta = builder.metadata();
    let total_rows = meta.file_metadata().num_rows().max(0) as u64;

    // 각 row group의 시작 행번호를 누적한다.
    let mut group_starts = Vec::with_capacity(meta.num_row_groups() + 1);
    let mut acc = 0u64;
    for i in 0..meta.num_row_groups() {
        group_starts.push(acc);
        acc += meta.row_group(i).num_rows().max(0) as u64;
    }
    group_starts.push(acc);

    let schema = builder.schema();
    let columns: Vec<String> = schema.fields().iter().map(|f| f.name().to_string()).collect();

    let numeric_cols = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            use arrow_schema::DataType as D;
            matches!(
                f.data_type(),
                D::Int8
                    | D::Int16
                    | D::Int32
                    | D::Int64
                    | D::UInt8
                    | D::UInt16
                    | D::UInt32
                    | D::UInt64
                    | D::Float16
                    | D::Float32
                    | D::Float64
                    | D::Decimal128(_, _)
                    | D::Decimal256(_, _)
            )
        })
        .map(|(i, _)| i)
        .collect();

    // GeoParquet: `geo` 키-값 메타데이터에서 geometry 컬럼 이름을 뽑아
    // 인덱스로 바꾼다. 없으면 빈 집합이고 바이너리는 길이 요약으로 나온다.
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
        numeric_cols,
        group_starts,
        visible: None,
        cache: Vec::new(),
    })
}

impl ParquetDoc {
    /// 파일의 **데이터** 행 수. 화면의 논리 행 수는 여기에 헤더 1을 더한 값이다
    /// (`app::doc_line_count` 참조).
    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    pub fn column_names(&self) -> &[String] {
        &self.columns
    }

    /// 이 컬럼이 숫자 타입인가. **Parquet은 타입이 확정적이라** 텍스트 경로의
    /// "숫자로 보이는지" 추론이 필요 없다 — 스키마가 답을 갖고 있다.
    pub fn column_is_numeric(&self, col: usize) -> bool {
        self.numeric_cols.contains(&col)
    }

    /// 화면에 보이는 컬럼을 지정한다. 컬럼 지향 포맷의 핵심 이점 — 100개 중
    /// 8개만 보이면 디코드 비용이 1/12이다. 집합이 바뀌면 캐시를 버린다.
    ///
    /// **찾기·정렬·내보내기는 이 값에 영향받으면 안 된다**(안 보이는 컬럼의
    /// 매치를 놓친다). `app::widen_parquet_to_all_columns`가 그 전에 None으로
    /// 되돌린다.
    pub fn set_visible_columns(&mut self, cols: Option<Vec<usize>>) {
        if self.visible != cols {
            self.visible = cols;
            // 여기서 비우는 것이 1차 방어다. `CachedGroup.cols`를 캐시 키에
            // 넣어 둔 것은 2차 방어로, 이 `clear`를 누가 지우거나 다른 경로가
            // `visible`을 직접 만져도 낡은 셀이 화면에 나오지 않게 한다
            // (`stale_cache_is_rejected_by_the_column_key`가 그 층을 직접
            //  겨눈다 — 변이 테스트에서 이 두 겹이 서로를 가려 한쪽을 지워도
            //  통과하는 것을 보고 각 층을 따로 검증하도록 갈랐다).
            self.cache.clear();
        }
    }

    /// `visible`만 바꾸고 캐시는 그대로 두었을 때, 컬럼 키가 낡은 항목을
    /// 걸러 내는지 확인하기 위한 테스트 전용 통로.
    #[cfg(test)]
    fn set_visible_without_clearing(&mut self, cols: Option<Vec<usize>>) {
        self.visible = cols;
    }

    /// 논리 행 → 한 줄 문자열.
    ///
    /// **인덱스 규약**: 논리 행 0은 **헤더**(컬럼 이름)이고, 논리 행 k(≥1)는
    /// 파일 행 k-1이다. 혼동하면 모든 행이 하나씩 밀린다.
    pub fn row_line(&mut self, logical: usize, delim: u8) -> Option<String> {
        if logical == 0 {
            let names: Vec<String> = match &self.visible {
                Some(v) => v
                    .iter()
                    .filter_map(|&i| self.columns.get(i))
                    .map(|c| sanitize_cell(c))
                    .collect(),
                None => self.columns.iter().map(|c| sanitize_cell(c)).collect(),
            };
            return Some(join_row(&names, delim));
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
    /// `visible`과 무관하다(정렬은 보이는 것과 상관없이 동작해야 한다).
    pub fn column_values(&mut self, col: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(self.total_rows as usize);
        for g in 0..self.group_starts.len().saturating_sub(1) {
            let rows = self.decode_group(g, Some(&[col]))?;
            for r in rows {
                out.push(r.into_iter().next().unwrap_or_default());
            }
        }
        Ok(out)
    }

    /// 문자열 컬럼의 평균 길이 추정. **첫 row group만 재서** 전체를 추정한다 —
    /// 푸터 통계가 아니라 실제 값을 재므로 대표성이 있고, 한 그룹이라 싸다.
    /// 숫자 컬럼이면 0(정렬 키가 고정 폭이라 길이가 의미 없다).
    pub fn estimated_avg_len(&mut self, col: usize) -> usize {
        if self.column_is_numeric(col) || self.group_starts.len() < 2 {
            return 0;
        }
        let Ok(rows) = self.decode_group(0, Some(&[col])) else {
            return 0;
        };
        if rows.is_empty() {
            return 0;
        }
        let total: usize = rows.iter().filter_map(|r| r.first()).map(|s| s.len()).sum();
        total / rows.len()
    }

    /// 파일 행이 속한 row group. `group_starts`가 오름차순이라 이진탐색.
    fn group_of(&self, file_row: u64) -> Option<usize> {
        if self.group_starts.len() < 2 {
            return None;
        }
        let last = self.group_starts.len() - 2;
        match self.group_starts.binary_search(&file_row) {
            Ok(i) => Some(i.min(last)),
            Err(i) => Some(i.saturating_sub(1).min(last)),
        }
    }

    /// 그룹이 캐시에 없으면 디코드해 넣는다. 가장 오래된 것을 앞에서 밀어낸다.
    fn ensure_group(&mut self, g: usize) -> Result<(), String> {
        if let Some(pos) = self
            .cache
            .iter()
            .position(|c| c.index == g && c.cols == self.visible)
        {
            // 최근 사용으로 올린다.
            let item = self.cache.remove(pos);
            self.cache.push(item);
            return Ok(());
        }
        // 컬럼 집합이 바뀐 같은 그룹의 낡은 캐시는 버린다.
        self.cache.retain(|c| c.index != g);
        let cols = self.visible.clone();
        let rows = self.decode_group(g, cols.as_deref())?;
        if self.cache.len() >= CACHE_GROUPS {
            self.cache.remove(0);
        }
        self.cache.push(CachedGroup {
            index: g,
            cols,
            rows,
        });
        Ok(())
    }

    /// row group 하나를 디코드해 행별 문자열 벡터로 만든다.
    /// `cols`가 있으면 그 컬럼만 읽는다(컬럼 프로젝션).
    fn decode_group(&self, g: usize, cols: Option<&[usize]>) -> Result<Vec<Vec<String>>, String> {
        use arrow_cast::display::{ArrayFormatter, FormatOptions};
        use parquet::arrow::ProjectionMask;

        let file = std::fs::File::open(&self.path).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| format!("Parquet 읽기 실패: {e}"))?;
        let mask = match cols {
            Some(c) => ProjectionMask::roots(builder.parquet_schema(), c.iter().copied()),
            None => ProjectionMask::all(),
        };
        let reader = builder
            .with_row_groups(vec![g])
            .with_projection(mask)
            .with_batch_size(BATCH_ROWS)
            .build()
            .map_err(|e| format!("row group {g} 디코드 실패: {e}"))?;

        // **프로젝션을 쓰면 배치의 컬럼 순서가 원본 스키마와 달라진다.**
        // geometry 판정은 반드시 원본 인덱스로 해야 한다.
        let orig_idx: Vec<usize> = match cols {
            Some(c) => c.to_vec(),
            None => (0..self.columns.len()).collect(),
        };

        let mut out = Vec::new();
        let opts = FormatOptions::default();
        for batch in reader {
            let b = batch.map_err(|e| format!("row group {g} 배치 실패: {e}"))?;
            // 포매터는 컬럼마다 한 번만 만든다(행마다 만들면 비싸다).
            let fmts: Vec<Option<ArrayFormatter>> = (0..b.num_columns())
                .map(|c| ArrayFormatter::try_new(b.column(c).as_ref(), &opts).ok())
                .collect();
            for r in 0..b.num_rows() {
                let mut cells = Vec::with_capacity(b.num_columns());
                for c in 0..b.num_columns() {
                    let col = b.column(c);
                    let oi = orig_idx.get(c).copied().unwrap_or(c);
                    // null은 빈 문자열이다 — `NULL`이라 쓰면 실제 문자열
                    // "NULL"과 구분되지 않고, CSV로 내보낼 때도 빈 값이 관행이다.
                    //
                    // **기본 `FormatOptions`도 null을 빈 문자열로 낸다**(실측).
                    // 그래도 이 분기를 명시적으로 두는 이유는, 그 동작이
                    // `FormatOptions`의 기본값에 딸린 것이라 라이브러리가
                    // 바꾸거나 우리가 옵션을 손대면 조용히 `NULL`/`<null>`이
                    // 새어 나오기 때문이다. 계약을 코드에 박아 둔다
                    // (`null_never_renders_as_the_literal_null_marker`).
                    if col.is_null(r) {
                        cells.push(String::new());
                        continue;
                    }
                    // 바이너리는 16진수 덤프(21바이트가 42자) 대신 요약으로.
                    let is_geo = self.geometry_cols.contains(&oi);
                    if let Some(bin) = col.as_any().downcast_ref::<arrow_array::BinaryArray>() {
                        cells.push(format_binary_cell(bin.value(r), is_geo));
                        continue;
                    }
                    if let Some(bin) = col.as_any().downcast_ref::<arrow_array::LargeBinaryArray>()
                    {
                        cells.push(format_binary_cell(bin.value(r), is_geo));
                        continue;
                    }
                    match &fmts[c] {
                        Some(f) => cells.push(sanitize_cell(&f.value(r).to_string())),
                        // 포맷할 수 없는 타입은 그 셀만 포기하고 계속 간다 —
                        // 한 컬럼 때문에 파일 전체를 못 보면 안 된다.
                        None => cells.push("<unsupported>".to_string()),
                    }
                }
                out.push(cells);
            }
        }
        Ok(out)
    }
}

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
    /// 규율 — 내용으로 이름을 지으면 같은 내용을 쓰는 병렬 테스트끼리 같은
    /// 파일을 잡고 경합한다.
    pub fn temp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_pq_{}_{}_{}.parquet", std::process::id(), tag, id));
        p
    }

    /// (id: int64, name: utf8) 두 컬럼짜리 Parquet. `names`의 None은 null.
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
        let names: ArrayRef = Arc::new(StringArray::from(
            (0..n).map(|i| format!("r{i}")).collect::<Vec<_>>(),
        ));
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

    /// little-endian POINT WKB를 만든다.
    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut v = vec![1u8, 1, 0, 0, 0];
        v.extend_from_slice(&x.to_le_bytes());
        v.extend_from_slice(&y.to_le_bytes());
        v
    }

    #[test]
    fn fixture_writes_a_readable_parquet_file() {
        let p = temp_path("fixture");
        write_simple(&p, vec![1, 2, 3], vec![Some("가"), None, Some("다")]);
        let bytes = std::fs::read(&p).unwrap();
        assert!(bytes.starts_with(b"PAR1"), "Parquet 매직으로 시작해야 한다");
        assert!(bytes.ends_with(b"PAR1"), "Parquet은 매직으로 끝나기도 한다");
        let _ = std::fs::remove_file(&p);
    }

    // ---- WKB 요약 파서 ----

    #[test]
    fn wkb_point_shows_coordinates() {
        let b = wkb_point(127.024, 37.512);
        assert_eq!(
            super::wkb_summary(&b).as_deref(),
            Some("POINT(127.024 37.512)")
        );
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
        assert_eq!(
            super::wkb_summary(&v).as_deref(),
            Some("MULTIPOLYGON(3 parts)")
        );
    }

    #[test]
    fn wkb_rejects_broken_input_instead_of_panicking() {
        assert_eq!(super::wkb_summary(&[]), None, "빈 입력");
        assert_eq!(super::wkb_summary(&[1, 1]), None, "너무 짧음");
        assert_eq!(super::wkb_summary(&[9, 1, 0, 0, 0]), None, "엔디안 코드 이상");
        assert_eq!(
            super::wkb_summary(&[1, 99, 0, 0, 0]),
            None,
            "타입 코드 범위 밖"
        );
        assert_eq!(super::wkb_summary(&[1, 1, 0, 0, 0]), None, "좌표 부족");
    }

    #[test]
    fn wkb_polygon_with_absurd_ring_count_does_not_overflow() {
        // 조작된 좌표 수(u32::MAX)로 오프셋 계산이 넘치면 패닉한다.
        let mut v = vec![1u8, 3, 0, 0, 0];
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(super::wkb_summary(&v), None, "포화 대신 None");
    }

    #[test]
    fn group_digits_inserts_thousand_separators() {
        assert_eq!(super::group_digits(0), "0");
        assert_eq!(super::group_digits(999), "999");
        assert_eq!(super::group_digits(1204), "1,204");
        assert_eq!(super::group_digits(1_000_000), "1,000,000");
    }

    // ---- 셀 포맷과 CSV 인용 ----

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
        // 탭 구분자면 콤마는 인용 대상이 아니다.
        assert_eq!(super::quote_cell("a,b", b'\t'), "a,b");
        assert_eq!(super::quote_cell("a\tb", b'\t'), "\"a\tb\"");
    }

    /// **설계의 핵심 계약**: `join_row`로 만든 줄을 `split_fields`가 원래
    /// 셀로 되돌린다. 이게 깨지면 표의 모든 컬럼이 어긋난다.
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
        assert_eq!(
            super::format_binary_cell(&[1, 99, 0, 0, 0], true),
            "<binary 5 B>"
        );
    }

    // ---- geo 메타데이터 ----

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
        // WKT는 `wkb_summary`로 읽을 수 없으므로 제외한다.
        let j = r#"{"columns":{"a":{"encoding":"WKT"},"b":{"encoding":"WKB"}}}"#;
        let got = super::geometry_columns(Some(j));
        assert!(!got.contains("a"), "WKT는 제외");
        assert!(got.contains("b"));
    }

    #[test]
    fn geometry_columns_is_empty_without_geo_metadata() {
        assert!(super::geometry_columns(None).is_empty(), "geo 키 없음");
    }

    // ---- ParquetDoc: 열기와 행 조회 ----

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
        // 인덱스 규약: 논리 행 k는 파일 행 k-1이다.
        let p = temp_path("rows");
        write_simple(&p, vec![10, 20], vec![Some("가"), Some("나")]);
        let mut d = super::open(&p).unwrap();
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
    fn null_never_renders_as_the_literal_null_marker() {
        // ArrayFormatter의 기본 출력은 `<null>`이다. 그대로 두면 실제 문자열
        // "NULL"과 구분되지 않고 CSV로 내보낼 때도 관행에 어긋난다.
        let p = temp_path("nullmark");
        write_simple(&p, vec![1], vec![None]);
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        assert!(!line.contains("null"), "null 마커가 새어 나왔다: {line}");
        assert_eq!(line, "1,");
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
    fn cells_containing_newlines_are_flattened_to_one_line() {
        let p = temp_path("nl");
        write_simple(&p, vec![1], vec![Some("첫줄\n둘째줄")]);
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        assert!(!line.contains('\n'), "개행이 남으면 행 정렬이 깨진다: {line}");
        assert_eq!(line, "1,첫줄 둘째줄");
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
        // POINT 요약에는 쉼표가 없어 인용되지 않는다. 중요한 것은 인용 여부가
        // 아니라 **왕복**이다 — 표가 이 줄을 되잘라 원래 셀을 얻어야 한다.
        assert_eq!(
            crate::parse::split_fields(&line, b','),
            vec!["POINT(127.024 37.512)".to_string(), "r0".to_string()]
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn polygon_summary_with_thousands_separator_round_trips() {
        // `POLYGON(1,204 pts)`에는 쉼표가 있어 인용이 필요하다. 인용이 없으면
        // 이 한 셀이 두 컬럼으로 갈려 표 전체가 밀린다.
        let p = temp_path("geopoly");
        // 링 1개에 좌표 1,204개.
        let mut poly = vec![1u8, 3, 0, 0, 0];
        poly.extend_from_slice(&1u32.to_le_bytes());
        poly.extend_from_slice(&1204u32.to_le_bytes());
        for i in 0..1204u32 {
            poly.extend_from_slice(&(i as f64).to_le_bytes());
            poly.extend_from_slice(&(i as f64).to_le_bytes());
        }
        let j = r#"{"columns":{"geometry":{"encoding":"WKB"}}}"#;
        write_with_geo(&p, vec![poly.as_slice()], Some(j));
        let mut d = super::open(&p).unwrap();
        let line = d.row_line(1, b',').unwrap();
        assert!(line.starts_with("\"POLYGON(1,204 pts)\""), "인용 필요: {line}");
        assert_eq!(
            crate::parse::split_fields(&line, b','),
            vec!["POLYGON(1,204 pts)".to_string(), "r0".to_string()],
            "쉼표가 든 요약이 한 셀로 유지돼야 한다"
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

    #[test]
    fn numeric_columns_are_known_from_the_schema() {
        // Parquet은 타입이 확정적이라 "숫자로 보이는지" 추론이 필요 없다.
        let p = temp_path("numeric");
        write_simple(&p, vec![1], vec![Some("가")]);
        let d = super::open(&p).unwrap();
        assert!(d.column_is_numeric(0), "id는 int64");
        assert!(!d.column_is_numeric(1), "name은 utf8");
        let _ = std::fs::remove_file(&p);
    }

    // ---- 정렬 ----

    #[test]
    fn sort_permutation_uses_one_based_logical_rows() {
        // **규약**: permutation[view_row] = 논리 행번호(1-based, 헤더 포함
        // 좌표계). 파일 행 f의 논리 행번호는 f+1이다. 여기를 틀리면 모든
        // 행이 하나씩 밀린다.
        let vals = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        // 오름차순: a(파일행1) < b(파일행2) < c(파일행0) → 논리행 2, 3, 1
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
        // 숫자: 9 < 10 < 100 → 파일행 1,0,2 → 논리행 2,1,3
        assert_eq!(super::sort_permutation(&vals, true, true), vec![2u32, 1, 3]);
        // 문자열이면 "10" < "100" < "9"
        assert_eq!(super::sort_permutation(&vals, false, true), vec![1u32, 3, 2]);
    }

    #[test]
    fn sort_permutation_puts_unparseable_numbers_last() {
        let vals = vec!["5".to_string(), "".to_string(), "1".to_string()];
        // 1(논리3), 5(논리1), 빈값(논리2)
        assert_eq!(super::sort_permutation(&vals, true, true), vec![3u32, 1, 2]);
    }

    #[test]
    fn sort_permutation_keeps_unparseable_last_even_when_descending() {
        // 방향과 무관하게 뒤 — 값 없는 행이 맨 뒤에 모이는 것이 읽기 쉽다.
        let vals = vec!["5".to_string(), "".to_string(), "1".to_string()];
        assert_eq!(super::sort_permutation(&vals, true, false), vec![1u32, 3, 2]);
    }

    #[test]
    fn sort_permutation_is_stable_for_equal_keys() {
        let vals = vec!["a".to_string(), "a".to_string(), "a".to_string()];
        assert_eq!(super::sort_permutation(&vals, false, true), vec![1u32, 2, 3]);
    }

    #[test]
    fn numeric_sort_estimate_is_twelve_bytes_per_row() {
        assert_eq!(super::estimate_sort_bytes(1000, true, 0), 12_000);
    }

    #[test]
    fn string_sort_estimate_includes_average_length() {
        // 문자열은 가변 길이라 12바이트 모델이 성립하지 않는다.
        assert_eq!(super::estimate_sort_bytes(1000, false, 20), 32_000);
    }

    #[test]
    fn sort_estimate_saturates_instead_of_overflowing() {
        // 넘치면 게이트가 작은 수를 보고 통과시킨다.
        assert_eq!(
            super::estimate_sort_bytes(u64::MAX, false, usize::MAX),
            u64::MAX
        );
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

    // ---- 컬럼 프로젝션 ----

    #[test]
    fn changing_visible_columns_invalidates_the_cache() {
        // 컬럼 집합을 캐시 키에 넣지 않으면 가로 스크롤 후 예전 집합으로
        // 디코드된 캐시가 그대로 쓰여 빈 셀이 나온다.
        let p = temp_path("proj");
        write_simple(&p, vec![7], vec![Some("가")]);
        let mut d = super::open(&p).unwrap();

        d.set_visible_columns(Some(vec![0]));
        assert_eq!(d.row_line(1, b',').as_deref(), Some("7"), "첫 컬럼만");

        d.set_visible_columns(Some(vec![1]));
        assert_eq!(
            d.row_line(1, b',').as_deref(),
            Some("가"),
            "두 번째 컬럼만 — 캐시가 무효화됐다"
        );

        d.set_visible_columns(None);
        assert_eq!(d.row_line(1, b',').as_deref(), Some("7,가"), "전체");
        let _ = std::fs::remove_file(&p);
    }

    /// 캐시 무효화는 두 겹이다: `set_visible_columns`의 `clear`(1차)와
    /// `CachedGroup.cols` 키(2차). 변이 테스트에서 두 겹이 서로를 가려
    /// **키를 지워도 통과**하는 것을 확인하고 추가한 테스트다 — 1차 방어를
    /// 우회해 2차 방어만 겨눈다.
    #[test]
    fn stale_cache_is_rejected_by_the_column_key() {
        let p = temp_path("stalekey");
        write_simple(&p, vec![7], vec![Some("가")]);
        let mut d = super::open(&p).unwrap();

        // 전체 컬럼으로 한 번 캐시를 채운다.
        assert_eq!(d.row_line(1, b',').as_deref(), Some("7,가"));

        // 캐시를 비우지 **않고** 보이는 컬럼만 바꾼다(1차 방어 우회).
        d.set_visible_without_clearing(Some(vec![1]));
        assert_eq!(
            d.row_line(1, b',').as_deref(),
            Some("가"),
            "컬럼 키가 낡은 캐시를 걸러 재디코드해야 한다"
        );
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
        assert_eq!(
            d.column_values(1).unwrap(),
            vec!["b", "a"],
            "안 보여도 읽는다"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn geometry_summary_survives_projection_reordering() {
        // 프로젝션을 쓰면 배치의 컬럼 순서가 원본과 달라진다. geometry
        // 판정을 배치 인덱스로 하면 엉뚱한 컬럼을 요약하게 된다.
        let p = temp_path("projgeo");
        let pt = wkb_point(1.0, 2.0);
        let j = r#"{"columns":{"geometry":{"encoding":"WKB"}}}"#;
        write_with_geo(&p, vec![pt.as_slice()], Some(j));
        let mut d = super::open(&p).unwrap();
        // geometry(0)만 보이게 하면 배치 인덱스도 0이라 우연히 맞는다.
        // name(1)만 보이게 하면 배치 인덱스 0이 name이므로, 원본 인덱스로
        // 판정하지 않으면 name을 geometry로 오인한다.
        d.set_visible_columns(Some(vec![1]));
        assert_eq!(d.row_line(1, b',').as_deref(), Some("r0"), "name은 그대로");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn geometry_summary_still_works_when_projected_alone() {
        let p = temp_path("projgeo2");
        let pt = wkb_point(1.0, 2.0);
        let j = r#"{"columns":{"geometry":{"encoding":"WKB"}}}"#;
        write_with_geo(&p, vec![pt.as_slice()], Some(j));
        let mut d = super::open(&p).unwrap();
        d.set_visible_columns(Some(vec![0]));
        assert_eq!(d.row_line(1, b',').as_deref(), Some("POINT(1 2)"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn geometry_columns_survives_broken_json() {
        // 깨진 JSON으로 파일 열기를 실패시키지 않는다 — geometry 표시는
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
}
