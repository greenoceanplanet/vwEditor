use crate::index::LineIndex;
use crate::indexer;
use crate::parse::{self, Encoding, SeparatorMode};
use crate::sort::{self, SortDir, SortKind, SortSpec};
use crate::source::{self, Source};
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;

/// 현재 적용된 정렬 상태. permutation[i] = 정렬 순서 i번째로 보여줄 원본 데이터
/// 행의 논리 행번호. col/kind/dir은 헤더 화살표/버튼 상태 표시에 쓴다(다중이면
/// 1차 기준). spec_count는 상태바에 "N개 기준" 표시용(1이면 단일).
pub struct SortState {
    pub permutation: Vec<u32>,
    pub col: usize,
    pub kind: SortKind,
    pub dir: SortDir,
    pub spec_count: usize,
}

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
    /// 헤더 클릭으로 선택된 컬럼(표 모드에서만). 정렬 대상.
    pub selected_col: Option<usize>,
    /// 현재 적용된 정렬. None이면 원본 순서.
    pub sort: Option<SortState>,
    /// 진행 중인 백그라운드 정렬 작업. 완료되면 sort로 옮기고 None이 된다.
    pub sort_job: Option<sort::SortJob>,
    /// 다중 컬럼 정렬 다이얼로그 표시 여부.
    pub show_sort_dialog: bool,
    /// 다이얼로그에서 편집 중인 정렬 기준 목록(위가 1차).
    pub sort_specs: Vec<SortSpec>,
}

pub struct App {
    pub doc: Option<Document>,
    pub error: Option<String>,
    /// 행 번호 시작값(0 또는 1). 표시 순번에 더해 라인번호로 쓴다.
    pub row_base: usize,
    /// 열 번호 시작값(0 또는 1). 컬럼 인덱스에 더해 헤더 번호로 쓴다.
    pub col_base: usize,
    /// 행/열 번호 설정 다이얼로그 표시 여부.
    pub show_numbering_dialog: bool,
}

impl Default for App {
    fn default() -> Self {
        App {
            doc: None,
            error: None,
            // 요청 기본값: 행/열 모두 0부터.
            row_base: 0,
            col_base: 0,
            show_numbering_dialog: false,
        }
    }
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
            selected_col: None,
            sort: None,
            sort_job: None,
            show_sort_dialog: false,
            sort_specs: Vec::new(),
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

        // 최상단 메뉴바 (파일 / 도구)
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("파일", |ui| {
                    if ui.button("열기…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.open_path(&path, ctx);
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("도구", |ui| {
                    // 도구 메뉴 항목은 파일이 열려 있을 때만 의미가 있다.
                    let has_doc = self.doc.is_some();
                    ui.add_enabled_ui(has_doc, |ui| {
                        if ui.button("다중 정렬…").clicked() {
                            if let Some(doc) = &mut self.doc {
                                // 표 모드 + 인덱싱 완료일 때만 실제로 연다.
                                let complete =
                                    doc.index.status().phase == crate::index::Phase::Complete;
                                if matches!(doc.sep, SeparatorMode::Char(_)) && complete {
                                    if doc.sort_specs.is_empty() {
                                        let col = doc.selected_col.unwrap_or(0);
                                        doc.sort_specs.push(SortSpec {
                                            col,
                                            kind: SortKind::Text,
                                            dir: SortDir::Asc,
                                        });
                                    }
                                    doc.show_sort_dialog = true;
                                }
                            }
                            ui.close_menu();
                        }
                        if ui.button("행/열 번호…").clicked() {
                            self.show_numbering_dialog = true;
                            ui.close_menu();
                        }
                    });
                });
            });
        });

        // 상단 툴바
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
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
                    let sep_before = doc.sep;
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
                    // 구분자가 바뀌면 컬럼 경계가 달라지므로 선택/정렬을 무효화한다.
                    if doc.sep != sep_before {
                        doc.sort = None;
                        doc.sort_job = None;
                        doc.selected_col = None;
                        // 컬럼 인덱스가 무의미해지므로 다중 정렬 기준도 초기화.
                        doc.sort_specs.clear();
                        doc.show_sort_dialog = false;
                    }
                    // 인코딩 드롭다운
                    let enc_before = doc.enc;
                    let enc_label = format!("{:?}", doc.enc);
                    egui::ComboBox::from_label("인코딩")
                        .selected_text(enc_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf8, "UTF-8");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Cp949, "CP949");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Le, "UTF-16LE");
                            ui.selectable_value(&mut doc.enc, crate::parse::Encoding::Utf16Be, "UTF-16BE");
                        });
                    // 인코딩/구분자가 바뀌면 파싱 기준이 달라지므로 정렬을 무효화한다.
                    if doc.enc != enc_before {
                        doc.sort = None;
                        doc.sort_job = None;
                    }
                    // 헤더 체크박스는 표 모드에서만 의미가 있다.
                    if matches!(doc.sep, SeparatorMode::Char(_)) {
                        let hdr_before = doc.has_header;
                        ui.checkbox(&mut doc.has_header, "헤더");
                        // 헤더 유무가 바뀌면 data_start가 달라져 permutation이 어긋나므로 무효화.
                        if doc.has_header != hdr_before {
                            doc.sort = None;
                            doc.sort_job = None;
                        }
                    }

                    // 정렬 컨트롤: 표 모드 + 컬럼 선택 + 인덱싱 완료일 때만 활성.
                    if matches!(doc.sep, SeparatorMode::Char(_)) {
                        ui.separator();
                        render_sort_controls(ui, doc, ctx);
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
                                // 재스캔으로 행 구성이 바뀔 수 있으므로 정렬 무효화.
                                doc.sort = None;
                                doc.sort_job = None;
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
                            // 정렬이 적용돼 있으면 어떤 기준인지 표시.
                            if let Some(s) = &doc.sort {
                                let kind = match s.kind {
                                    SortKind::Text => "문자",
                                    SortKind::Number => "숫자",
                                };
                                let dir = match s.dir {
                                    SortDir::Asc => "오름차순",
                                    SortDir::Desc => "내림차순",
                                };
                                ui.separator();
                                if s.spec_count > 1 {
                                    ui.label(format!(
                                        "{}개 기준 정렬됨 (1차: {}번 컬럼)",
                                        s.spec_count,
                                        s.col + 1
                                    ));
                                } else {
                                    ui.label(format!("{}번 컬럼 {kind} {dir} 정렬됨", s.col + 1));
                                }
                            }
                        }
                    }
                } else {
                    ui.label("파일을 여세요");
                }
            });
        });

        // 본문: 구분 모드에 따라 표 뷰 / 텍스트 뷰로 분기.
        let row_base = self.row_base;
        let col_base = self.col_base;
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(doc) = &mut self.doc else { return };
            match doc.sep {
                SeparatorMode::Char(delim) => render_table(ui, doc, delim, row_base, col_base),
                SeparatorMode::None => render_text(ui, &*doc, row_base),
            }
        });

        // 다중 컬럼 정렬 다이얼로그(표시 중일 때만).
        if let Some(doc) = &mut self.doc {
            if doc.show_sort_dialog {
                render_sort_dialog(ctx, doc, col_base);
            }
        }

        // 행/열 번호 설정 다이얼로그.
        if self.show_numbering_dialog {
            render_numbering_dialog(ctx, self);
        }
    }
}

/// 행/열 번호 시작값(0 또는 1) 설정 다이얼로그.
fn render_numbering_dialog(ctx: &egui::Context, app: &mut App) {
    let mut open = true;
    egui::Window::new("행/열 번호")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("행/열 번호를 몇부터 시작할지 정합니다.");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("행 번호:");
                ui.selectable_value(&mut app.row_base, 0, "0부터");
                ui.selectable_value(&mut app.row_base, 1, "1부터");
            });
            ui.horizontal(|ui| {
                ui.label("열 번호:");
                ui.selectable_value(&mut app.col_base, 0, "0부터");
                ui.selectable_value(&mut app.col_base, 1, "1부터");
            });
            ui.separator();
            if ui.button("닫기").clicked() {
                app.show_numbering_dialog = false;
            }
        });
    if !open {
        app.show_numbering_dialog = false;
    }
}

/// 다중 컬럼 정렬 다이얼로그. 정렬 기준(컬럼·문자/숫자·오름/내림) 목록을
/// 위(1차)→아래(N차) 순으로 편집하고, "정렬"로 백그라운드 다중 정렬을 시작한다.
fn render_sort_dialog(ctx: &egui::Context, doc: &mut Document, col_base: usize) {
    let delim = match doc.sep {
        SeparatorMode::Char(d) => d,
        SeparatorMode::None => {
            doc.show_sort_dialog = false;
            return;
        }
    };
    let data_start = if doc.has_header { 1 } else { 0 };

    // 컬럼 수: 헤더가 있으면 헤더 필드 수, 없으면 첫 데이터 행 필드 수로 근사.
    let col_count = {
        let probe = if doc.has_header { 0 } else { data_start };
        parse_logical_line(doc, probe, delim)
            .map(|f| f.len())
            .unwrap_or(1)
            .max(1)
    };
    // 헤더 이름(드롭다운 라벨용).
    let header_fields: Option<Vec<String>> = if doc.has_header {
        parse_logical_line(doc, 0, delim)
    } else {
        None
    };
    let col_label = |c: usize| -> String {
        // 표시 번호는 col_base(0 또는 1)를 반영.
        let n = c + col_base;
        match &header_fields {
            Some(h) => format!("{} {}", n, h.get(c).cloned().unwrap_or_default()),
            None => format!("{n}번 컬럼"),
        }
    };

    let mut open = true;
    let mut do_sort = false;
    let mut remove_idx: Option<usize> = None;

    egui::Window::new("다중 컬럼 정렬")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label("위에 있는 기준이 1차 정렬입니다. 위→아래 순으로 적용됩니다.");
            ui.separator();

            for i in 0..doc.sort_specs.len() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}순위", i + 1));

                    // 컬럼 선택 드롭다운.
                    let cur_col = doc.sort_specs[i].col.min(col_count - 1);
                    egui::ComboBox::from_id_source(("sortcol", i))
                        .selected_text(col_label(cur_col))
                        .show_ui(ui, |ui| {
                            for c in 0..col_count {
                                ui.selectable_value(&mut doc.sort_specs[i].col, c, col_label(c));
                            }
                        });

                    // 문자/숫자.
                    egui::ComboBox::from_id_source(("sortkind", i))
                        .selected_text(match doc.sort_specs[i].kind {
                            SortKind::Text => "문자",
                            SortKind::Number => "숫자",
                        })
                        .width(56.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.sort_specs[i].kind, SortKind::Text, "문자");
                            ui.selectable_value(
                                &mut doc.sort_specs[i].kind,
                                SortKind::Number,
                                "숫자",
                            );
                        });

                    // 오름/내림.
                    egui::ComboBox::from_id_source(("sortdir", i))
                        .selected_text(match doc.sort_specs[i].dir {
                            SortDir::Asc => "오름차순",
                            SortDir::Desc => "내림차순",
                        })
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut doc.sort_specs[i].dir, SortDir::Asc, "오름차순");
                            ui.selectable_value(&mut doc.sort_specs[i].dir, SortDir::Desc, "내림차순");
                        });

                    // 삭제(기준이 2개 이상일 때만).
                    if doc.sort_specs.len() > 1 && ui.button("✖").clicked() {
                        remove_idx = Some(i);
                    }
                });
            }

            ui.separator();
            ui.horizontal(|ui| {
                // 기준 추가(MAX_KEYS 미만일 때).
                if doc.sort_specs.len() < sort::MAX_KEYS && ui.button("+ 기준 추가").clicked() {
                    let col = doc.sort_specs.last().map(|s| s.col).unwrap_or(0);
                    doc.sort_specs.push(SortSpec {
                        col,
                        kind: SortKind::Text,
                        dir: SortDir::Asc,
                    });
                }
                if doc.sort_specs.len() >= sort::MAX_KEYS {
                    ui.label(format!("(최대 {}개)", sort::MAX_KEYS));
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("정렬").clicked() {
                    do_sort = true;
                }
                if ui.button("취소").clicked() {
                    doc.show_sort_dialog = false;
                }
            });
        });

    if let Some(i) = remove_idx {
        doc.sort_specs.remove(i);
    }
    // 창 X로 닫아도 다이얼로그 종료.
    if !open {
        doc.show_sort_dialog = false;
    }

    if do_sort && !doc.sort_specs.is_empty() {
        doc.show_sort_dialog = false;
        doc.sort_job = Some(sort::spawn_multi_sort(
            doc.source.clone(),
            doc.index.clone(),
            doc.enc,
            delim,
            doc.sort_specs.clone(),
            data_start,
            ctx.clone(),
        ));
    }
}

/// 툴바의 정렬 컨트롤. 컬럼이 선택돼 있고 인덱싱이 완료(Phase::Complete)일 때만
/// 정렬 버튼이 활성화된다. 정렬은 백그라운드 스레드에서 수행되며, 진행 중에는
/// progress bar가 표시되고 완료되면 permutation을 doc.sort로 옮긴다.
fn render_sort_controls(ui: &mut egui::Ui, doc: &mut Document, ctx: &egui::Context) {
    use crate::index::Phase;

    // 진행 중인 정렬 작업이 끝났는지 먼저 폴링해 결과를 수거.
    if let Some(job) = &mut doc.sort_job {
        if let Some(perm) = job.take_result() {
            let spec_count = job.specs.len().max(1);
            doc.sort = Some(SortState {
                permutation: perm,
                col: job.col,
                kind: job.kind,
                dir: job.dir,
                spec_count,
            });
            doc.sort_job = None;
        }
    }

    // 정렬 진행 중이면 progress bar만 표시하고 버튼은 숨긴다.
    if let Some(job) = &doc.sort_job {
        let p = job.progress();
        ui.label("정렬 중");
        ui.add(
            egui::ProgressBar::new(p)
                .desired_width(160.0)
                .text(format!("{:.0}%", p * 100.0)),
        );
        return;
    }

    let complete = doc.index.status().phase == Phase::Complete;
    let selected = doc.selected_col;

    match selected {
        Some(col) => ui.label(format!("정렬: {}번 컬럼", col + 1)),
        None => ui.label("정렬: (헤더 클릭해 컬럼 선택)"),
    };

    let delim = match doc.sep {
        SeparatorMode::Char(d) => d,
        SeparatorMode::None => return,
    };
    let data_start = if doc.has_header { 1 } else { 0 };

    // 컬럼 미선택 또는 인덱싱 미완료면 버튼 비활성.
    let can_sort = selected.is_some() && complete;

    let mut do_sort: Option<(SortKind, SortDir)> = None;
    ui.add_enabled_ui(can_sort, |ui| {
        if ui.button("문자↑").clicked() {
            do_sort = Some((SortKind::Text, SortDir::Asc));
        }
        if ui.button("문자↓").clicked() {
            do_sort = Some((SortKind::Text, SortDir::Desc));
        }
        if ui.button("숫자↑").clicked() {
            do_sort = Some((SortKind::Number, SortDir::Asc));
        }
        if ui.button("숫자↓").clicked() {
            do_sort = Some((SortKind::Number, SortDir::Desc));
        }
    });

    // 다중 컬럼 정렬 다이얼로그 열기(인덱싱 완료일 때만).
    ui.add_enabled_ui(complete, |ui| {
        if ui.button("다중 정렬…").clicked() {
            // 다이얼로그를 열 때 기준 목록이 비어 있으면 현재 선택 컬럼(있으면)으로
            // 첫 기준을 미리 채워 사용자가 바로 편집하게 한다.
            if doc.sort_specs.is_empty() {
                let col = doc.selected_col.unwrap_or(0);
                doc.sort_specs.push(SortSpec {
                    col,
                    kind: SortKind::Text,
                    dir: SortDir::Asc,
                });
            }
            doc.show_sort_dialog = true;
        }
    });

    // 정렬 해제 버튼은 정렬이 적용돼 있을 때만.
    if doc.sort.is_some() && ui.button("정렬 해제").clicked() {
        doc.sort = None;
    }

    if !complete && selected.is_some() {
        ui.label("(인덱싱 완료 후 정렬 가능)");
    }

    // 정렬 버튼이 눌리면 백그라운드 작업을 띄운다.
    if let (Some((kind, dir)), Some(col)) = (do_sort, selected) {
        doc.sort_job = Some(sort::spawn_sort(
            doc.source.clone(),
            doc.index.clone(),
            doc.enc,
            delim,
            col,
            data_start,
            kind,
            dir,
            ctx.clone(),
        ));
    }
}

/// 표 모드 렌더: 라인번호 + 구분자로 분리한 필드 컬럼들.
/// 헤더 클릭으로 컬럼을 선택하고, 정렬이 적용돼 있으면 permutation 순서로 렌더.
fn render_table(ui: &mut egui::Ui, doc: &mut Document, delim: u8, row_base: usize, col_base: usize) {
    use std::cell::Cell;

    // 헤더 행 데이터(있으면 첫 줄)와 데이터 시작 행 결정
    let total_lines = doc.index.line_count();
    let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
        parse_logical_line(doc, 0, delim)
    } else {
        None
    };

    let data_start = if doc.has_header { 1 } else { 0 };
    let data_rows = total_lines.saturating_sub(data_start);

    // 클로저에서 doc를 불변으로만 빌리기 위해, 렌더에 필요한 상태를 미리 뽑는다.
    let selected_col = doc.selected_col;
    // 정렬된 컬럼과 방향(헤더 화살표 표시용).
    let sorted_col_dir: Option<(usize, SortDir)> =
        doc.sort.as_ref().map(|s| (s.col, s.dir));
    // permutation은 참조로만 사용(클론 방지). 정렬 시 행 순서 매핑에 씀.
    let permutation: Option<&[u32]> = doc.sort.as_ref().map(|s| s.permutation.as_slice());
    // 헤더 클릭으로 새로 선택된 컬럼을 클로저 밖으로 전달하는 통로.
    let clicked_col: Cell<Option<usize>> = Cell::new(None);

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
                    let selected = selected_col == Some(c);
                    // 셀 전체 영역을 클릭 대상으로 만든다(글자뿐 아니라 헤더 칸
                    // 어디를 눌러도 선택되도록). 먼저 셀의 남은 사각형 전체에
                    // 클릭 sense 응답을 만든다.
                    let cell_rect = ui.max_rect();
                    let resp = ui.interact(
                        cell_rect,
                        ui.id().with(("hdr", c)),
                        egui::Sense::click(),
                    );
                    // 선택된 컬럼은 헤더 칸 전체에 밝은 파란 음영.
                    if selected {
                        ui.painter().rect_filled(
                            cell_rect,
                            0.0,
                            egui::Color32::from_rgb(60, 110, 180),
                        );
                    }
                    // 헤더 텍스트: "번호 이름" + 정렬 화살표. 번호는 col_base 반영.
                    let cn = c + col_base;
                    let base = if let Some(h) = &header_fields {
                        format!("{} {}", cn, h.get(c).cloned().unwrap_or_default())
                    } else {
                        format!("{cn}")
                    };
                    let arrow = match sorted_col_dir {
                        Some((sc, SortDir::Asc)) if sc == c => " ↑",
                        Some((sc, SortDir::Desc)) if sc == c => " ↓",
                        _ => "",
                    };
                    let rich = egui::RichText::new(format!("{base}{arrow}")).strong();
                    ui.add(egui::Label::new(rich).truncate());
                    if resp.clicked() {
                        clicked_col.set(Some(c));
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, data_rows, |mut row| {
                let view_row = row.index();
                let line_no = view_row + row_base;
                // 정렬이 적용돼 있으면 permutation으로 원본 논리 행번호를 얻는다.
                // 없으면 원본 순서(data_start + view_row).
                let logical = match permutation {
                    Some(perm) => perm.get(view_row).map(|&r| r as usize),
                    None => Some(data_start + view_row),
                };
                // 라인번호 컬럼 — 화면 순번(정렬 후, row_base부터). 원본 행번호가
                // 아니라 보이는 순서를 매겨 스크롤 위치 감각을 유지한다.
                row.col(|ui| {
                    ui.add(egui::Label::new(format!("{line_no}")).truncate());
                });
                let fields = logical
                    .and_then(|l| parse_logical_line(doc, l, delim))
                    .unwrap_or_default();
                for c in 0..col_count {
                    row.col(|ui| {
                        // 선택된 컬럼은 셀 배경에 밝은 파란 음영(줄무늬 위에 반투명).
                        if selected_col == Some(c) {
                            ui.painter().rect_filled(
                                ui.max_rect(),
                                0.0,
                                egui::Color32::from_rgba_unmultiplied(80, 150, 230, 70),
                            );
                        }
                        ui.add(
                            egui::Label::new(fields.get(c).cloned().unwrap_or_default())
                                .truncate(),
                        );
                    });
                }
            });
        });

    // 클로저 종료 후 헤더 클릭 결과를 반영(같은 컬럼 재클릭이면 선택 해제 토글).
    if let Some(c) = clicked_col.get() {
        doc.selected_col = if doc.selected_col == Some(c) {
            None
        } else {
            Some(c)
        };
    }
}

/// 텍스트 모드 렌더: 라인번호 + 줄 전체(구분 안 함). 긴 줄은 truncate하고
/// 컬럼 폭을 넓히면(가로 스크롤) 전체가 보인다.
fn render_text(ui: &mut egui::Ui, doc: &Document, row_base: usize) {
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
                let line_no = logical + row_base;
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
