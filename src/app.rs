use crate::index::LineIndex;
use crate::indexer;
use crate::parse::{self, Encoding, SeparatorMode};
use crate::source::{self, Source};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct Document {
    pub source: Arc<Source>,
    pub index: LineIndex,
    pub enc: Encoding,
    pub sep: SeparatorMode,
    pub has_header: bool,
    pub indexer: Option<JoinHandle<()>>,
    pub path_label: String,
    /// 툴바 "직접 입력" 커스텀 구분자 텍스트박스의 현재 값(한 글자).
    pub custom_sep_input: String,
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
        let sep = parse::detect_separator(path, &head_lines);
        // 헤더 감지는 표 모드일 때만 의미가 있다. 텍스트 모드면 헤더 없음.
        let has_header = match sep {
            SeparatorMode::Char(d) => {
                let sample_rows: Vec<Vec<String>> = head_lines
                    .iter()
                    .take(20)
                    .map(|l| parse::split_fields(l, d))
                    .collect();
                parse::detect_header(&sample_rows)
            }
            SeparatorMode::None => false,
        };

        let index = LineIndex::new(src.len());
        let handle = indexer::spawn_indexer(src.clone(), index.clone(), enc, ctx.clone());

        // 커스텀 입력란 초기값: 감지된 구분자가 화면 표시 가능한 ASCII면 그 글자,
        // 아니면 빈 문자열.
        let custom_sep_input = match sep {
            SeparatorMode::Char(b) if b.is_ascii_graphic() => (b as char).to_string(),
            _ => String::new(),
        };

        self.doc = Some(Document {
            source: src,
            index,
            enc,
            sep,
            has_header,
            indexer: Some(handle),
            path_label: path.display().to_string(),
            custom_sep_input,
        });
    }
}

use crate::index::Phase;
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 22.0;

/// 논리 행 번호(logical)에 해당하는 줄을 offset으로 조회해 디코딩·개행 제거한
/// 문자열 하나를 돌려준다(구분자 분리 없음). 텍스트 모드 렌더와, 표 모드의
/// 필드 분리(`parse_logical_line`)가 공유하는 디코딩 단계.
/// 해당 논리 행이 인덱스에 없으면(범위 밖 등) `None`.
fn decode_logical_line(doc: &Document, logical: usize) -> Option<String> {
    doc.index.line_range(logical).map(|(s, e)| {
        crate::parse::decode_line(doc.source.slice(s, e), doc.enc)
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    })
}

/// 논리 행을 디코딩한 뒤 구분자 `delim`으로 필드 분리한다. 표 모드(SeparatorMode::Char)
/// 전용. 헤더 행, col_count 샘플링, 데이터 행 렌더링이 모두 이 함수를 공유한다.
fn parse_logical_line(doc: &Document, logical: usize, delim: u8) -> Option<Vec<String>> {
    decode_logical_line(doc, logical).map(|text| crate::parse::split_fields(&text, delim))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ctrl + 휠로 전역 확대/축소. zoom_factor를 조절하면 폰트뿐 아니라
        // UI 전체가 배율에 맞춰 커지고 작아진다. (0.5배 ~ 4.0배로 제한)
        let scroll_y = ctx.input(|i| {
            if i.modifiers.ctrl || i.modifiers.command {
                i.raw_scroll_delta.y
            } else {
                0.0
            }
        });
        if scroll_y != 0.0 {
            let factor = ctx.zoom_factor();
            // 휠 한 칸(대략 ±? px)마다 배율을 곱셈으로 조절해 부드럽게.
            let new_factor = (factor * (1.0 + scroll_y * 0.001)).clamp(0.5, 4.0);
            ctx.set_zoom_factor(new_factor);
        }

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
                    // 구분자 드롭다운. None(텍스트) + 표준 구분자들 + 직접 입력.
                    let sep_label = match doc.sep {
                        SeparatorMode::None => "구분 안 함(텍스트)".to_owned(),
                        SeparatorMode::Char(b',') => "콤마 ,".to_owned(),
                        SeparatorMode::Char(b'\t') => "탭".to_owned(),
                        SeparatorMode::Char(b'|') => "파이프 |".to_owned(),
                        SeparatorMode::Char(b';') => "세미콜론 ;".to_owned(),
                        SeparatorMode::Char(b) if b.is_ascii_graphic() => {
                            format!("직접: {}", b as char)
                        }
                        SeparatorMode::Char(b) => format!("직접: 0x{b:02X}"),
                    };
                    egui::ComboBox::from_label("구분자")
                        .selected_text(sep_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.sep, SeparatorMode::None, "구분 안 함(텍스트)");
                            ui.selectable_value(&mut doc.sep, SeparatorMode::Char(b','), "콤마 ,");
                            ui.selectable_value(&mut doc.sep, SeparatorMode::Char(b'\t'), "탭");
                            ui.selectable_value(&mut doc.sep, SeparatorMode::Char(b'|'), "파이프 |");
                            ui.selectable_value(&mut doc.sep, SeparatorMode::Char(b';'), "세미콜론 ;");
                        });
                    // 직접 입력: 한 글자 텍스트박스. 입력하면 그 글자(첫 바이트)를
                    // 구분자로 사용. ASCII 한 글자만 유효(멀티바이트는 첫 바이트).
                    ui.label("직접:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut doc.custom_sep_input)
                            .desired_width(28.0)
                            .char_limit(1),
                    );
                    if resp.changed() {
                        if let Some(&b) = doc.custom_sep_input.as_bytes().first() {
                            doc.sep = SeparatorMode::Char(b);
                        }
                        // 입력을 비우면 텍스트 모드로.
                        if doc.custom_sep_input.is_empty() {
                            doc.sep = SeparatorMode::None;
                        }
                    }
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
                    // 헤더 체크박스는 표 모드에서만 의미가 있다.
                    if matches!(doc.sep, SeparatorMode::Char(_)) {
                        ui.checkbox(&mut doc.has_header, "헤더");
                    }
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
                                "중단됨 — 앞부분 {} 행 표시 중 ({done_gb:.2} / {total_gb:.2} GB)",
                                doc.index.line_count()
                            ));
                            if ui.button("이어서 읽기").clicked() {
                                // 재개 = 처음부터 다시 병렬 스캔. spawn_indexer가
                                // 프라이밍→병렬을 새로 수행하며 인덱스를 덮어쓴다.
                                // 기존 핸들은 이미 종료됨.
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

        // 본문: 구분 모드에 따라 표 뷰 / 텍스트 뷰로 분기.
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(doc) = &self.doc else { return };
            match doc.sep {
                SeparatorMode::Char(delim) => render_table(ui, doc, delim),
                SeparatorMode::None => render_text(ui, doc),
            }
        });
    }
}

/// 표 모드 렌더: 라인번호 + 구분자로 분리한 필드 컬럼들.
fn render_table(ui: &mut egui::Ui, doc: &Document, delim: u8) {
    // 헤더 행 데이터(있으면 첫 줄)와 데이터 시작 행 결정
    let total_lines = doc.index.line_count();
    let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
        parse_logical_line(doc, 0, delim)
    } else {
        None
    };

    let data_start = if doc.has_header { 1 } else { 0 };
    let data_rows = total_lines.saturating_sub(data_start);

    // col_count는 헤더 필드 수와, 앞부분 데이터 행 몇 개를 샘플링한 필드 수의
    // 최댓값으로 정한다. 헤더가 없는 파일(header_fields == None)에서 1로
    // 고정되어 컬럼이 다 숨는 문제, 그리고 헤더보다 넓은 행이 잘리는 문제를
    // 함께 해결한다.
    const COL_COUNT_SAMPLE_ROWS: usize = 10;
    let mut col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(0);
    for logical in data_start..data_start + COL_COUNT_SAMPLE_ROWS {
        if let Some(fields) = parse_logical_line(doc, logical, delim) {
            col_count = col_count.max(fields.len());
        }
    }
    let col_count = col_count.max(1);

    // 테이블이 남은 세로 공간을 모두 채우도록 한다.
    // - max_scroll_height 기본값(800px)이 스크롤 영역을 제한해 창을 키워도
    //   ~35행에서 멈추므로, 사용 가능한 높이로 올린다.
    // - auto_shrink의 y축을 false로 두어 내용이 적어도 테이블이 창을 채운다.
    let avail_height = ui.available_height();

    // 컬럼은 auto()(전 행 measure로 대용량에서 느림) 대신 고정 초기폭 +
    // 드래그 조절(resizable)로 둔다. 긴 값은 셀에서 truncate 되고 폭을
    // 넓히면 전체가 보인다.
    let table = TableBuilder::new(ui)
        .striped(true)
        .auto_shrink([false, false])
        .max_scroll_height(avail_height)
        .column(Column::initial(64.0).at_least(48.0).resizable(true)) // 라인번호 #
        .columns(Column::initial(120.0).at_least(60.0).resizable(true), col_count);

    table
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.add(egui::Label::new(egui::RichText::new("#").strong()).truncate());
            });
            for c in 0..col_count {
                header.col(|ui| {
                    let text = if let Some(h) = &header_fields {
                        format!("{} {}", c + 1, h.get(c).cloned().unwrap_or_default())
                    } else {
                        format!("{}", c + 1)
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, data_rows, |mut row| {
                let logical = row.index() + data_start;
                let line_no = row.index() + 1;
                // 라인번호 컬럼 — 한 줄 고정(긴 값으로 인한 wrap 방지)
                row.col(|ui| {
                    ui.add(egui::Label::new(format!("{line_no}")).truncate());
                });
                // 데이터 컬럼들 — 이 행만 offset으로 조회·디코딩·파싱
                let fields = parse_logical_line(doc, logical, delim).unwrap_or_default();
                for c in 0..col_count {
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(fields.get(c).cloned().unwrap_or_default())
                                .truncate(),
                        );
                    });
                }
            });
        });
}

/// 텍스트 모드 렌더: 라인번호 + 줄 전체(구분 안 함). 긴 줄은 truncate하고
/// 컬럼 폭을 넓히면(가로 스크롤) 전체가 보인다.
fn render_text(ui: &mut egui::Ui, doc: &Document) {
    let total_lines = doc.index.line_count();
    let avail_height = ui.available_height();

    // 줄 전체 컬럼은 넉넉한 초기폭 + resizable. 긴 줄은 셀 안에서 truncate.
    let table = TableBuilder::new(ui)
        .striped(true)
        .auto_shrink([false, false])
        .max_scroll_height(avail_height)
        .column(Column::initial(64.0).at_least(48.0).resizable(true)) // 라인번호 #
        .column(Column::remainder().at_least(200.0).resizable(true)); // 줄 전체

    table
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.add(egui::Label::new(egui::RichText::new("#").strong()).truncate());
            });
            header.col(|ui| {
                ui.add(egui::Label::new(egui::RichText::new("내용").strong()).truncate());
            });
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, total_lines, |mut row| {
                let logical = row.index();
                let line_no = logical + 1;
                row.col(|ui| {
                    ui.add(egui::Label::new(format!("{line_no}")).truncate());
                });
                let line = decode_logical_line(doc, logical).unwrap_or_default();
                row.col(|ui| {
                    ui.add(egui::Label::new(line).truncate());
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(content: &[u8]) -> std::path::PathBuf {
        temp_ext(content, "csv")
    }

    /// 확장자를 지정해 임시 파일을 만든다. detect_separator가 확장자를 먼저
    /// 보므로, 내용 기반 감지를 테스트하려면 중립적 확장자(txt 등)를 써야 한다.
    fn temp_ext(content: &[u8], ext: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("tv_app_{}_{}.{ext}", std::process::id(), id));
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
        assert_eq!(doc.sep, SeparatorMode::Char(b','));
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

    /// update()의 col_count 계산 로직을 GUI 없이 그대로 재현해 검증하는 헬퍼.
    /// 렌더 코드(render_table)와 동일한 공식을 사용한다. 표 모드 전용이므로
    /// doc.sep이 Char임을 가정하고 그 delim을 꺼내 쓴다.
    fn compute_col_count(doc: &Document) -> usize {
        let delim = match doc.sep {
            SeparatorMode::Char(d) => d,
            SeparatorMode::None => return 1,
        };
        let total_lines = doc.index.line_count();
        let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
            parse_logical_line(doc, 0, delim)
        } else {
            None
        };
        let data_start = if doc.has_header { 1 } else { 0 };
        const COL_COUNT_SAMPLE_ROWS: usize = 10;
        let mut col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(0);
        for logical in data_start..data_start + COL_COUNT_SAMPLE_ROWS {
            if let Some(fields) = parse_logical_line(doc, logical, delim) {
                col_count = col_count.max(fields.len());
            }
        }
        col_count.max(1)
    }

    #[test]
    fn headerless_ragged_file_col_count_matches_widest_row() {
        // has_header가 false로 판정되도록 전부 숫자인 CSV(detect_header가 false 반환).
        // 첫 줄은 4개, 이후 5개 필드짜리 행 포함 → col_count는 4가 아니라 5여야 함
        // (샘플링이 헤더 없는 파일에서 실제 데이터 폭을 반영하는지 검증).
        let p = temp(b"1,2,3,4\n5,6,7,8,9\n10,11,12,13\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        // 인덱싱을 완료까지 join해 line_count/line_range가 안정적으로 채워지게 한다.
        doc.indexer.take().unwrap().join().unwrap();

        assert!(!doc.has_header, "전부 숫자인 파일은 헤더 없음으로 판정되어야 함");
        assert_eq!(compute_col_count(doc), 5);
    }

    #[test]
    fn headerless_uniform_file_col_count_matches_field_count() {
        // 브리프에 명시된 케이스: 헤더 없는 4열 파일 → col_count는 1이 아니라 4.
        let p = temp(b"1,2,3,4\n5,6,7,8\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();

        assert!(!doc.has_header);
        assert_eq!(compute_col_count(doc), 4);
    }

    #[test]
    fn plain_text_opens_in_text_mode() {
        // 구분자가 거의 없는 산문 → SeparatorMode::None(텍스트 모드)로 열려야 하고,
        // 헤더는 꺼져 있어야 한다. decode_logical_line은 줄 전체를 돌려준다.
        let p = temp_ext(
            b"The quick brown fox\njumps over the lazy dog\nand runs away fast\n",
            "txt",
        );
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();

        assert_eq!(doc.sep, SeparatorMode::None, "구분자 없는 텍스트는 None으로");
        assert!(!doc.has_header, "텍스트 모드는 헤더 없음");
        // 각 논리 행이 줄 전체(구분/분리 없이) 그대로 디코딩되는지.
        assert_eq!(
            decode_logical_line(doc, 0).as_deref(),
            Some("The quick brown fox")
        );
        assert_eq!(
            decode_logical_line(doc, 1).as_deref(),
            Some("jumps over the lazy dog")
        );
    }

    #[test]
    fn custom_separator_splits_fields() {
        // 커스텀 구분자(물결 ~)로 필드 분리가 되는지. detect는 표준 후보만 보므로
        // 이 파일은 None으로 열리지만, 사용자가 sep을 Char(b'~')로 바꾸면
        // parse_logical_line이 ~로 분리해야 한다.
        let p = temp_ext(b"a~b~c\nd~e~f\n", "txt");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        doc.sep = SeparatorMode::Char(b'~');

        assert_eq!(
            parse_logical_line(doc, 0, b'~'),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
        assert_eq!(compute_col_count(doc), 3);
    }
}
