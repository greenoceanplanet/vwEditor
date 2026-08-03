//! Parquet 읽기 전용 뷰어의 순수 로직 — 셀 포맷, WKB 요약, 행 조립, 캐시.
//! egui 없음. `hex.rs`/`find.rs`/`convert.rs`와 같은 규율이다.
//!
//! **왜 읽기 전용인가.** Parquet은 컬럼별로 흩어진 압축·인코딩 포맷이라
//! 셀 하나를 고치려면 페이지를 풀어 다시 인코딩·압축해야 하고, 크기가
//! 달라져 뒤따르는 모든 청크의 오프셋이 밀리며 푸터 메타데이터를 전부
//! 재계산해야 한다. 사실상 파일 전체 재작성이다. 텍스트 파일에서
//! "수정분만 반영해 저장"이 가능한 것은 바이트 오프셋이 안정적이기
//! 때문인데, Parquet에는 그 성질이 없다.

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
