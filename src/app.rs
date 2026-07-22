use crate::index::LineIndex;
use crate::indexer;
use crate::parse::{self, Encoding};
use crate::source::{self, Source};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct Document {
    pub source: Arc<Source>,
    pub index: LineIndex,
    pub enc: Encoding,
    pub delim: u8,
    pub has_header: bool,
    pub indexer: Option<JoinHandle<()>>,
    pub path_label: String,
}

#[derive(Default)]
pub struct App {
    pub doc: Option<Document>,
    pub error: Option<String>,
}

/// 프라이밍 시 감지에 쓸 앞부분 바이트 크기.
const PRIME_BYTES: usize = 64 * 1024;

impl App {
    /// 파일을 열고 앞부분으로 인코딩/구분자/헤더를 감지한 뒤 백그라운드 인덱싱을 시작한다.
    pub fn open_path(&mut self, path: &Path, ctx: &egui::Context) {
        self.error = None;
        let src = match source::open(path) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.error = Some(format!("파일 열기 실패: {e}"));
                self.doc = None;
                return;
            }
        };

        let head = {
            let n = (src.len() as usize).min(PRIME_BYTES);
            src.slice(0, n as u64)
        };
        let enc = parse::detect_encoding(head);

        // 앞부분을 줄 단위로 나눠 구분자/헤더 감지에 사용
        let head_text = parse::decode_line(head, enc);
        let head_lines: Vec<&str> = head_text.lines().take(50).collect();
        let delim = parse::detect_delimiter(path, &head_lines);
        let sample_rows: Vec<Vec<String>> = head_lines
            .iter()
            .take(20)
            .map(|l| parse::split_fields(l, delim))
            .collect();
        let has_header = parse::detect_header(&sample_rows);

        let index = LineIndex::new(src.len());
        let handle = indexer::spawn_indexer(src.clone(), index.clone(), enc, ctx.clone());

        self.doc = Some(Document {
            source: src,
            index,
            enc,
            delim,
            has_header,
            indexer: Some(handle),
            path_label: path.display().to_string(),
        });
    }
}

use crate::index::Phase;
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 20.0;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 상단 툴바
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("파일 열기").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.open_path(&path, ctx);
                    }
                }
                if let Some(doc) = &mut self.doc {
                    ui.separator();
                    // 구분자 드롭다운
                    let delim_label = match doc.delim {
                        b',' => "콤마 ,",
                        b'\t' => "탭",
                        b'|' => "파이프 |",
                        b';' => "세미콜론 ;",
                        _ => "기타",
                    };
                    egui::ComboBox::from_label("구분자")
                        .selected_text(delim_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.delim, b',', "콤마 ,");
                            ui.selectable_value(&mut doc.delim, b'\t', "탭");
                            ui.selectable_value(&mut doc.delim, b'|', "파이프 |");
                            ui.selectable_value(&mut doc.delim, b';', "세미콜론 ;");
                        });
                    // 인코딩 드롭다운
                    let enc_label = format!("{:?}", doc.enc);
                    egui::ComboBox::from_label("인코딩")
                        .selected_text(enc_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf8, "UTF-8");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Cp949, "CP949");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Le, "UTF-16LE");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Be, "UTF-16BE");
                        });
                    ui.checkbox(&mut doc.has_header, "헤더");
                    ui.separator();
                    ui.label(&doc.path_label);
                }
            });
        });

        // 하단 상태바
        egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::RED, err);
                } else if let Some(doc) = &mut self.doc {
                    let st = doc.index.status();
                    let done_gb = st.bytes_done as f64 / 1e9;
                    let total_gb = st.total_bytes as f64 / 1e9;
                    let pct = if st.total_bytes > 0 {
                        st.bytes_done * 100 / st.total_bytes
                    } else {
                        100
                    };
                    match st.phase {
                        Phase::Priming | Phase::Indexing => {
                            ui.label(format!(
                                "인덱싱 중... {done_gb:.2} / {total_gb:.2} GB ({pct}%)"
                            ));
                            if ui.button("중단").clicked() {
                                doc.index.request_pause();
                            }
                        }
                        Phase::Paused => {
                            ui.label(format!(
                                "일시정지 — {done_gb:.2} / {total_gb:.2} GB ({pct}%)"
                            ));
                            if ui.button("이어서 읽기").clicked() {
                                // 새 워커 spawn(재개). 기존 핸들은 이미 종료됨.
                                let handle = crate::indexer::spawn_indexer(
                                    doc.source.clone(),
                                    doc.index.clone(),
                                    doc.enc,
                                    ctx.clone(),
                                );
                                doc.indexer = Some(handle);
                            }
                        }
                        Phase::Complete => {
                            ui.label(format!("완료 — {} 행", doc.index.line_count()));
                        }
                    }
                } else {
                    ui.label("파일을 여세요");
                }
            });
        });

        // 본문 테이블
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(doc) = &self.doc else { return };

            // 헤더 행 데이터(있으면 첫 줄)와 데이터 시작 행 결정
            let total_lines = doc.index.line_count();
            let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
                doc.index.line_range(0).map(|(s, e)| {
                    let text = crate::parse::decode_line(doc.source.slice(s, e), doc.enc);
                    crate::parse::split_fields(text.trim_end_matches(['\r', '\n']), doc.delim)
                })
            } else {
                None
            };

            let data_start = if doc.has_header { 1 } else { 0 };
            let data_rows = total_lines.saturating_sub(data_start);
            let col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(1).max(1);

            let table = TableBuilder::new(ui)
                .striped(true)
                .column(Column::auto().at_least(48.0)) // 라인번호 #
                .columns(Column::auto().at_least(60.0).resizable(true), col_count);

            table
                .header(ROW_HEIGHT, |mut header| {
                    header.col(|ui| {
                        ui.strong("#");
                    });
                    for c in 0..col_count {
                        header.col(|ui| {
                            if let Some(h) = &header_fields {
                                ui.strong(format!("{} {}", c + 1, h.get(c).cloned().unwrap_or_default()));
                            } else {
                                ui.strong(format!("{}", c + 1));
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, data_rows, |mut row| {
                        let logical = row.index() + data_start;
                        let line_no = row.index() + 1;
                        // 라인번호 컬럼
                        row.col(|ui| {
                            ui.label(format!("{line_no}"));
                        });
                        // 데이터 컬럼들 — 이 행만 offset으로 조회·디코딩·파싱
                        let fields = doc
                            .index
                            .line_range(logical)
                            .map(|(s, e)| {
                                let text = crate::parse::decode_line(doc.source.slice(s, e), doc.enc);
                                crate::parse::split_fields(
                                    text.trim_end_matches(['\r', '\n']),
                                    doc.delim,
                                )
                            })
                            .unwrap_or_default();
                        for c in 0..col_count {
                            row.col(|ui| {
                                ui.label(fields.get(c).cloned().unwrap_or_default());
                            });
                        }
                    });
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(content: &[u8]) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tv_app_{}.csv", content.len()));
        std::fs::File::create(&p).unwrap().write_all(content).unwrap();
        p
    }

    #[test]
    fn open_detects_and_primes() {
        let p = temp(b"name,age\nAlice,30\nBob,25\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_ref().unwrap();
        assert_eq!(doc.enc, crate::parse::Encoding::Utf8);
        assert_eq!(doc.delim, b',');
        assert!(doc.has_header);
        assert!(app.error.is_none());
    }

    #[test]
    fn open_missing_sets_error() {
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(std::path::Path::new("nope_xyz.csv"), &ctx);
        assert!(app.doc.is_none());
        assert!(app.error.is_some());
    }
}
