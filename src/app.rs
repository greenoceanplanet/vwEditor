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
    /// 실제 파일 경로. 저장(덮어쓰기)의 대상. `path_label`은 표시용 문자열이라
    /// 되파싱하지 않고 별도로 보관한다.
    pub path: std::path::PathBuf,
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
    /// 편집 모드 인메모리 버퍼. None이면 뷰 전용(mmap).
    pub edit: Option<crate::edit::EditBuffer>,
    /// 셀 편집 중인 위치와 편집 텍스트(표 모드).
    pub editing_cell: Option<(usize, usize)>,
    pub cell_edit_text: String,
    /// 셀 사각 선택(표 모드): (r0,c0,r1,c1) 논리 행/열.
    pub cell_sel: Option<(usize, usize, usize, usize)>,
    /// 드래그 선택이 셀에서 시작됐는지. 표 밖에서 누른 채 들어온 포인터가
    /// 선택을 끌고 가는 것을 막는다(egui의 primary_down은 전역 상태라
    /// 그것만으로는 시작 지점을 알 수 없다).
    pub cell_drag_active: bool,
    /// 텍스트 선택(텍스트 모드): (anchor, caret).
    pub text_sel: Option<(crate::edit::TextPos, crate::edit::TextPos)>,
    /// 텍스트 커서(텍스트 모드).
    pub text_caret: crate::edit::TextPos,
    /// 드래그 선택이 텍스트 줄에서 시작됐는지. `cell_drag_active`와 같은 이유로
    /// 필요하다 — egui의 primary_down은 전역 상태라 누름이 어디서 시작됐는지
    /// 알 수 없고, is_pointer_button_down_on()은 우클릭에도 참이다.
    pub text_drag_active: bool,
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
    /// 저장 다이얼로그 표시 + 편집 대상 인코딩/BOM 선택 상태.
    pub show_save_dialog: bool,
    pub save_as: bool,
    pub save_enc: crate::parse::Encoding,
    pub save_bom: bool,
    /// 셀 복사/잘라내기 시 채우는 앱 내부 클립보드. egui 0.28은 시스템
    /// 클립보드 읽기를 직접 제공하지 않으므로(붙여넣기는 이벤트로만 들어온다),
    /// 우클릭 "붙여넣기"의 소스로 이 캐시를 쓴다. 복사 시 시스템 클립보드에도
    /// 같은 내용을 넣어 외부 앱으로의 복사는 정상 동작한다.
    pub clipboard_cache: String,
    /// 저장하지 않은 변경이 있어 확인을 기다리는 동작. Some이면 확인 다이얼로그를
    /// 띄우고, 사용자가 "계속"을 누르면 그 동작을 수행한다.
    pub pending_action: Option<PendingAction>,
}

/// dirty 편집 버퍼를 잃을 수 있어 확인이 필요한 동작.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    /// 편집 모드 종료(버퍼 폐기).
    ExitEditMode,
    /// 다른 파일 열기(경로는 이미 고른 상태).
    OpenFile(std::path::PathBuf),
    /// 창 닫기(X / Alt+F4). 확인되면 실제로 `ViewportCommand::Close`를 보낸다.
    CloseApp,
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
            show_save_dialog: false,
            save_as: false,
            save_enc: crate::parse::Encoding::Utf8,
            save_bom: false,
            clipboard_cache: String::new(),
            pending_action: None,
        }
    }
}

/// 프라이밍 시 감지에 쓸 앞부분 바이트 크기.
const PRIME_BYTES: usize = 64 * 1024;

impl App {
    /// 저장하지 않은 편집 내용이 있는지.
    pub fn edit_dirty(&self) -> bool {
        self.doc
            .as_ref()
            .and_then(|d| d.edit.as_ref())
            .map_or(false, |e| e.dirty)
    }

    /// 저장 다이얼로그를 열 때 인코딩/BOM 기본값을 현재 문서 기준으로 맞춘다.
    /// (원본과 같은 인코딩으로 저장하는 것이 기본 기대 동작.)
    fn init_save_defaults(&mut self) {
        if let Some(doc) = &self.doc {
            self.save_enc = doc.enc;
            // CP949는 BOM이 없다. 나머지는 UTF-16이면 BOM을 기본 켬(없으면
            // 엔디안 판정이 불가능해 재열기가 깨진다).
            self.save_bom = matches!(
                doc.enc,
                crate::parse::Encoding::Utf16Le | crate::parse::Encoding::Utf16Be
            );
        }
    }

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
            path: path.to_path_buf(),
            path_label: path.display().to_string(),
            custom_sep_input,
            selected_col: None,
            sort: None,
            sort_job: None,
            show_sort_dialog: false,
            sort_specs: Vec::new(),
            edit: None,
            editing_cell: None,
            cell_edit_text: String::new(),
            cell_sel: None,
            cell_drag_active: false,
            text_sel: None,
            text_caret: crate::edit::TextPos { line: 0, col: 0 },
            text_drag_active: false,
        });
    }
}

use crate::index::Phase;
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 22.0;

/// 선택 음영(컬럼 선택·셀 사각 선택 공통). 줄무늬 위에 덧그리는 반투명 파랑.
/// `from_rgba_unmultiplied`가 const가 아니라 함수로 둔다.
fn sel_shade() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(80, 150, 230, 70)
}

/// 논리 행 번호(logical)에 해당하는 줄을 mmap offset으로 조회해 디코딩·개행
/// 제거한 문자열 하나를 돌려준다(구분자 분리 없음). **뷰 전용 경로**로,
/// 편집 버퍼를 보지 않는다 — 두 모드를 모두 다루는 것은 `logical_line`이다.
/// 해당 논리 행이 인덱스에 없으면(범위 밖 등) `None`.
fn decode_logical_line(doc: &Document, logical: usize) -> Option<String> {
    doc.index.line_range(logical).map(|(s, e)| {
        crate::parse::decode_line(doc.source.slice(s, e), doc.enc)
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    })
}

/// 편집 모드로 진입: 파일 전체를 현재 인코딩으로 줄 배열 로드.
/// (동기 로드 — 큰 파일 백그라운드화는 Task 9에서.)
pub fn enter_edit_mode(doc: &mut Document) {
    if doc.edit.is_some() {
        return;
    }
    let buf = crate::edit::load_edit_buffer(&doc.source, doc.enc);
    doc.edit = Some(buf);
    // 편집 모드에선 뷰 permutation 정렬을 폐기(이제 lines가 진실).
    doc.sort = None;
    doc.sort_job = None;
    doc.editing_cell = None;
    doc.cell_sel = None;
    doc.cell_drag_active = false;
    doc.text_sel = None;
    doc.text_caret = crate::edit::TextPos { line: 0, col: 0 };
    doc.text_drag_active = false;
}

/// 저장 직후 문서의 뷰 소스(mmap)를 방금 쓴 파일로 다시 겨눈다.
///
/// **왜 필요한가.** `Source`는 `Document`가 사는 동안 `Mmap`을 붙들고 있는데
/// (`source.rs`), `save::write_file`은 임시 파일을 만든 뒤 `std::fs::rename`으로
/// 원본을 갈아치운다(`save.rs`). Windows에서 이 rename 자체는 **성공**하지만,
/// 기존 매핑은 고아가 된 옛 파일 오브젝트의 **저장 전 바이트를 계속 돌려준다**.
/// 그대로 두면 저장 후 편집 모드를 껐을 때 `logical_line`이 `decode_logical_line`
/// (mmap 경로)으로 떨어지면서 화면이 **저장 전 내용**으로 되돌아가 사용자에게는
/// 작업이 날아간 것처럼 보인다. `index`도 낡아 행 수가 파일과 어긋난다.
/// "다른 이름으로 저장"이면 `path`만 새 파일을 가리키고 `source`는 원본을 매핑한
/// 채로 남아 더 나쁘다.
///
/// **왜 `open_path`가 아닌가.** `open_path`는 인코딩/구분자/헤더를 새로 감지하고
/// `Document`를 통째로 갈아끼운다 — 편집 버퍼(`edit`), 선택(`cell_sel`/`text_sel`),
/// 커서, 선택 컬럼이 전부 날아가고, 저장 인코딩이 원본과 다르면 툴바 설정까지
/// 바뀐다. 저장은 "파일이 갱신됐다"는 사건일 뿐 "새 파일을 열었다"가 아니므로,
/// 여기서는 `source` + `index` + 인덱서만 교체하고 나머지 편집 세션 상태는
/// **손대지 않는다**. `edit.lines`는 편집 모드의 진실이므로 절대 다시 읽지 않는다.
///
/// 실패(파일이 곧바로 지워졌다 등)하면 소스를 교체하지 않고 에러 문자열을
/// 돌려준다 — 낡은 매핑이 남지만 저장 자체는 이미 성공했고, 편집 버퍼가
/// 여전히 진실이므로 편집 모드 화면은 정확하다.
fn repoint_source_after_save(
    doc: &mut Document,
    path: &Path,
    ctx: &egui::Context,
) -> Result<(), String> {
    let src = match source::open(path) {
        Ok(s) => Arc::new(s),
        Err(e) => return Err(format!("저장 후 파일 다시 열기 실패: {e}")),
    };
    // 새 인덱스를 만들고 인덱서를 새로 띄운다(Paused → "이어서 읽기"와 같은 패턴).
    let index = LineIndex::new(src.len());
    let handle = indexer::spawn_indexer(src.clone(), index.clone(), doc.enc, ctx.clone());
    doc.source = src;
    doc.index = index;
    doc.indexer = Some(handle);
    // 옛 인덱스 기준의 permutation은 새 파일에 맞지 않는다.
    doc.sort = None;
    doc.sort_job = None;
    Ok(())
}

/// 편집 모드 이탈(버퍼 폐기). dirty 경고는 호출측 UI에서.
pub fn exit_edit_mode(doc: &mut Document) {
    doc.edit = None;
    doc.editing_cell = None;
    doc.cell_sel = None;
    doc.cell_drag_active = false;
    doc.text_sel = None;
    doc.text_drag_active = false;
}

/// logical 논리 행의 텍스트. 편집 모드면 EditBuffer에서, 아니면 mmap 디코딩.
pub fn logical_line(doc: &Document, logical: usize) -> Option<String> {
    if let Some(e) = &doc.edit {
        e.lines.get(logical).cloned()
    } else {
        decode_logical_line(doc, logical)
    }
}

/// `logical_line`(편집 모드 대응) 경유로 디코딩한 뒤 구분자 `delim`으로 필드
/// 분리한다. 표 모드(SeparatorMode::Char) 전용. `render_table`의 헤더/col_count
/// 샘플/데이터 셀과 `render_sort_dialog`의 컬럼 목록이 모두 이 함수를 공유한다 —
/// 편집 모드에서도 화면과 정렬 대상이 같은 내용을 보게 하는 것이 핵심이다.
fn parse_logical_line_edit(doc: &Document, logical: usize, delim: u8) -> Option<Vec<String>> {
    logical_line(doc, logical).map(|t| crate::parse::split_fields(&t, delim))
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

        // 창 닫기(X / Alt+F4). 저장하지 않은 편집이 있으면 닫기를 취소하고 다른
        // 폐기 경로(편집 모드 Off, 파일 → 열기…)와 같은 확인 창으로 보낸다.
        // 확인 창에서 "계속"을 누르면 그때 실제로 Close를 보낸다.
        if ctx.input(|i| i.viewport().close_requested()) {
            // 이미 확인 창이 떠 있으면(사용자가 X를 또 눌렀다) 중복 처리하지 않고
            // 닫기만 막는다 — pending_action을 덮어써 앞선 동작을 잃지 않게.
            if self.pending_action.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.edit_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.pending_action = Some(PendingAction::CloseApp);
            }
            // dirty가 아니면 그대로 닫히게 둔다.
        }

        // 최상단 메뉴바 (파일 / 도구)
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("파일", |ui| {
                    if ui.button("열기…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            // 저장하지 않은 변경이 있으면 확인 후에 연다.
                            if self.edit_dirty() {
                                self.pending_action = Some(PendingAction::OpenFile(path));
                            } else {
                                self.open_path(&path, ctx);
                            }
                        }
                        ui.close_menu();
                    }
                    // 저장 항목은 편집 모드일 때만 의미가 있다(뷰 모드는 버퍼가 없다).
                    let editing = self.doc.as_ref().map_or(false, |d| d.edit.is_some());
                    ui.add_enabled_ui(editing, |ui| {
                        if ui.button("저장").clicked() {
                            self.show_save_dialog = true;
                            self.save_as = false;
                            self.init_save_defaults();
                            ui.close_menu();
                        }
                        if ui.button("다른 이름으로 저장…").clicked() {
                            self.show_save_dialog = true;
                            self.save_as = true;
                            self.init_save_defaults();
                            ui.close_menu();
                        }
                    });
                });
                ui.menu_button("도구", |ui| {
                    // 도구 메뉴 항목은 파일이 열려 있을 때만 의미가 있다.
                    let has_doc = self.doc.is_some();
                    ui.add_enabled_ui(has_doc, |ui| {
                        // 편집 모드 토글. 켜면 파일 전체를 인메모리 버퍼로 읽고,
                        // 끄면 버퍼를 버린다(dirty면 확인 후).
                        let mut edit_on = self.doc.as_ref().map_or(false, |d| d.edit.is_some());
                        if ui.checkbox(&mut edit_on, "편집 모드").clicked() {
                            if edit_on {
                                if let Some(doc) = &mut self.doc {
                                    enter_edit_mode(doc);
                                }
                            } else if self.edit_dirty() {
                                self.pending_action = Some(PendingAction::ExitEditMode);
                            } else if let Some(doc) = &mut self.doc {
                                exit_edit_mode(doc);
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("다중 정렬…").clicked() {
                            if let Some(doc) = &mut self.doc {
                                // 표 모드 + 인덱싱 완료일 때만 실제로 연다.
                                // 편집 모드는 버퍼가 파일 전체를 이미 담고 있으므로
                                // 인덱싱 진행 상태와 무관하게 정렬할 수 있다.
                                let complete = doc.edit.is_some()
                                    || doc.index.status().phase == crate::index::Phase::Complete;
                                if matches!(doc.sep, SeparatorMode::Char(_)) && complete {
                                    if doc.sort_specs.is_empty() {
                                        let col = doc.selected_col.unwrap_or(0);
                                        doc.sort_specs.push(SortSpec {
                                            col,
                                            kind: SortKind::Text,
                                            dir: SortDir::Asc,
                                            ci: true,
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
                // 오류는 문서 상태와 **배타 분기가 아니라 덧붙는 구간**이다.
                // 저장 실패 순간이야말로 "편집 중 — N 행 / ● 변경됨"이 가장
                // 필요한 때인데, 예전처럼 else-if로 두면 그 표시가 사라졌다.
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::RED, err);
                    if self.doc.is_some() {
                        ui.separator();
                    }
                }
                if let Some(doc) = &mut self.doc {
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
                    // 편집 모드 표시. 인덱싱 단계와 무관하게 항상 보여야 하므로
                    // match 밖에 둔다. dirty면 붉은 "● 변경됨"을 덧붙인다.
                    if let Some(e) = &doc.edit {
                        ui.separator();
                        ui.label(format!("편집 중 — {} 행", e.lines.len()));
                        if e.dirty {
                            ui.colored_label(egui::Color32::from_rgb(230, 120, 60), "● 변경됨");
                        }
                    }
                } else if self.error.is_none() {
                    // 문서도 오류도 없을 때만 안내 문구.
                    ui.label("파일을 여세요");
                }
            });
        });

        // 본문: 구분 모드에 따라 표 뷰 / 텍스트 뷰로 분기.
        let row_base = self.row_base;
        let col_base = self.col_base;
        // 클립보드 캐시는 render_table이 복사/붙여넣기에 쓰므로 가변 대여를
        // doc과 분리해 넘긴다(App 전체를 넘기면 doc과 동시 대여가 불가능).
        let clipboard = &mut self.clipboard_cache;
        let doc_opt = &mut self.doc;
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(doc) = doc_opt else { return };
            match doc.sep {
                SeparatorMode::Char(delim) => {
                    render_table(ui, doc, delim, row_base, col_base, clipboard)
                }
                SeparatorMode::None => render_text(ui, doc, row_base, clipboard),
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

        // 저장 다이얼로그.
        if self.show_save_dialog {
            render_save_dialog(ctx, self);
        }

        // 저장하지 않은 변경 확인 다이얼로그.
        if self.pending_action.is_some() {
            render_confirm_discard_dialog(ctx, self);
        }

        // Ctrl+S — 편집 모드에서 저장 다이얼로그 열기. 다른 다이얼로그가 떠
        // 있으면 무시한다(중복 열기 방지).
        if self.doc.as_ref().map_or(false, |d| d.edit.is_some())
            && !self.show_save_dialog
            && self.pending_action.is_none()
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
            })
        {
            self.show_save_dialog = true;
            self.save_as = false;
            self.init_save_defaults();
        }
    }
}

/// 저장 다이얼로그. 인코딩/BOM을 고르고 저장하거나 취소한다.
/// `app.save_as`가 참이면 rfd 파일 선택 창으로 경로를 새로 고른다.
fn render_save_dialog(ctx: &egui::Context, app: &mut App) {
    // 편집 버퍼가 없으면(편집 모드 이탈 등) 다이얼로그를 닫는다.
    if app.doc.as_ref().map_or(true, |d| d.edit.is_none()) {
        app.show_save_dialog = false;
        return;
    }
    let title = if app.save_as { "다른 이름으로 저장" } else { "저장" };
    let cur_label = app
        .doc
        .as_ref()
        .map(|d| d.path_label.clone())
        .unwrap_or_default();

    let mut open = true;
    let mut do_save = false;
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            if app.save_as {
                ui.label("저장을 누르면 파일 위치를 고릅니다.");
            } else {
                ui.label(format!("덮어쓸 파일: {cur_label}"));
            }
            ui.separator();

            let enc_label = match app.save_enc {
                Encoding::Utf8 => "UTF-8",
                Encoding::Cp949 => "CP949",
                Encoding::Utf16Le => "UTF-16LE",
                Encoding::Utf16Be => "UTF-16BE",
            };
            egui::ComboBox::from_label("인코딩")
                .selected_text(enc_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.save_enc, Encoding::Utf8, "UTF-8");
                    ui.selectable_value(&mut app.save_enc, Encoding::Cp949, "CP949");
                    ui.selectable_value(&mut app.save_enc, Encoding::Utf16Le, "UTF-16LE");
                    ui.selectable_value(&mut app.save_enc, Encoding::Utf16Be, "UTF-16BE");
                });

            // CP949는 BOM 개념이 없으므로 체크박스를 비활성 + 강제 해제.
            let bom_allowed = app.save_enc != Encoding::Cp949;
            if !bom_allowed {
                app.save_bom = false;
            }
            ui.add_enabled_ui(bom_allowed, |ui| {
                ui.checkbox(&mut app.save_bom, "BOM 포함");
            });
            if !bom_allowed {
                ui.label("(CP949는 BOM이 없습니다)");
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("저장").clicked() {
                    do_save = true;
                }
                if ui.button("취소").clicked() {
                    app.show_save_dialog = false;
                }
            });
        });
    if !open {
        app.show_save_dialog = false;
    }

    if !do_save {
        return;
    }
    app.show_save_dialog = false;

    // 대상 경로 결정. save_as면 파일 선택 창, 아니면 현재 경로.
    // 현재 경로가 비어 있으면(있을 수 없지만 방어) save_as로 폴백한다.
    let cur_path = app.doc.as_ref().map(|d| d.path.clone()).unwrap_or_default();
    let target = if app.save_as || cur_path.as_os_str().is_empty() {
        let mut dlg = rfd::FileDialog::new();
        if let Some(dir) = cur_path.parent() {
            dlg = dlg.set_directory(dir);
        }
        if let Some(name) = cur_path.file_name().and_then(|n| n.to_str()) {
            dlg = dlg.set_file_name(name);
        }
        match dlg.save_file() {
            Some(p) => p,
            // 취소 = 아무 일도 일어나지 않는다(버퍼는 그대로 dirty).
            None => return,
        }
    } else {
        cur_path
    };

    let save_as = app.save_as;
    let opts = crate::save::SaveOptions {
        enc: app.save_enc,
        bom: app.save_bom,
        newline: app
            .doc
            .as_ref()
            .and_then(|d| d.edit.as_ref())
            .map(|e| e.newline)
            .unwrap_or(crate::edit::Newline::Lf),
    };
    let result = {
        let Some(e) = app.doc.as_ref().and_then(|d| d.edit.as_ref()) else { return };
        crate::save::write_file(&target, &e.lines, &opts, None)
    };

    match result {
        Ok(()) => {
            app.error = None;
            if let Some(doc) = &mut app.doc {
                if let Some(e) = &mut doc.edit {
                    e.dirty = false;
                }
                if save_as {
                    doc.path_label = target.display().to_string();
                    doc.path = target.clone();
                }
                // write_file의 rename으로 옛 mmap이 낡았다. 방금 쓴 파일로 다시
                // 겨눠야 편집 모드를 껐을 때 저장된 내용이 보인다. 편집 버퍼와
                // 선택 상태는 그대로 유지된다(사용자는 편집 모드에 남는다).
                if let Err(msg) = repoint_source_after_save(doc, &target, ctx) {
                    app.error = Some(msg);
                }
            }
        }
        Err(err) => {
            // 실패하면 버퍼는 dirty인 채로 둔다(사용자가 다시 시도할 수 있게).
            app.error = Some(format!("저장 실패: {err}"));
        }
    }
}

/// 저장하지 않은 변경을 버릴 수 있는 동작 전에 띄우는 확인 창.
fn render_confirm_discard_dialog(ctx: &egui::Context, app: &mut App) {
    let mut open = true;
    let mut proceed = false;
    let mut cancel = false;
    egui::Window::new("저장하지 않은 변경")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("저장하지 않은 변경이 있습니다. 계속하면 변경 내용을 잃습니다.");
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("계속").clicked() {
                    proceed = true;
                }
                if ui.button("취소").clicked() {
                    cancel = true;
                }
            });
        });
    // 창 X로 닫으면 취소와 같다(데이터를 잃지 않는 쪽이 기본).
    if !open || cancel {
        app.pending_action = None;
        return;
    }
    if !proceed {
        return;
    }
    match app.pending_action.take() {
        Some(PendingAction::ExitEditMode) => {
            if let Some(doc) = &mut app.doc {
                exit_edit_mode(doc);
            }
        }
        Some(PendingAction::OpenFile(p)) => {
            app.open_path(&p, ctx);
        }
        Some(PendingAction::CloseApp) => {
            // 확인됐으니 실제로 닫는다. 이번엔 close_requested 훅이 dirty를
            // 다시 보지 않도록 편집 버퍼의 dirty를 내려 둔다(이미 폐기 동의).
            if let Some(e) = app.doc.as_mut().and_then(|d| d.edit.as_mut()) {
                e.dirty = false;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        None => {}
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
    // 편집 모드에서도 올바른 내용을 보도록 `parse_logical_line_edit`을 쓴다 —
    // mmap 전용 `decode_logical_line` 경로는 편집 전 원본을 읽어 값이 낡는다.
    let col_count = {
        let probe = if doc.has_header { 0 } else { data_start };
        parse_logical_line_edit(doc, probe, delim)
            .map(|f| f.len())
            .unwrap_or(1)
            .max(1)
    };
    // 헤더 이름(드롭다운 라벨용).
    let header_fields: Option<Vec<String>> = if doc.has_header {
        parse_logical_line_edit(doc, 0, delim)
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
    // 순서 변경(위/아래로 한 칸): (from, to). 클로저 종료 후 swap.
    let mut swap_pair: Option<(usize, usize)> = None;

    egui::Window::new("다중 컬럼 정렬")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label("위에 있는 기준이 1차 정렬입니다. 위→아래 순으로 적용됩니다.");
            ui.separator();

            // 각 기준이 현재 선택 중인 컬럼 목록(스냅샷). 드롭다운에서 "다른 행이
            // 이미 쓰는 컬럼"을 제외해 같은 컬럼 중복 선택을 막는다.
            let selected_cols: Vec<usize> = doc.sort_specs.iter().map(|s| s.col).collect();

            for i in 0..doc.sort_specs.len() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}순위", i + 1));

                    // 컬럼 선택 드롭다운. 다른 기준이 이미 선택한 컬럼은 목록에서 제외.
                    let cur_col = doc.sort_specs[i].col.min(col_count - 1);
                    egui::ComboBox::from_id_source(("sortcol", i))
                        .selected_text(col_label(cur_col))
                        .show_ui(ui, |ui| {
                            for c in 0..col_count {
                                // 이 행(i) 자신의 현재 값은 남기고, 다른 행이 쓰는
                                // 컬럼만 숨긴다.
                                let used_by_other = selected_cols
                                    .iter()
                                    .enumerate()
                                    .any(|(j, &sc)| j != i && sc == c);
                                if used_by_other {
                                    continue;
                                }
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

                    // 대소문자 무시(문자 기준일 때만). 체크됨 = 무시(ci=true).
                    if doc.sort_specs[i].kind == SortKind::Text {
                        ui.checkbox(&mut doc.sort_specs[i].ci, "대소문자 무시");
                    }

                    // 순서 변경(↑ 위로, ↓ 아래로). 맨 위/맨 아래에선 해당 버튼 비활성.
                    let n = doc.sort_specs.len();
                    ui.add_enabled_ui(i > 0, |ui| {
                        if ui.button("↑").clicked() {
                            swap_pair = Some((i, i - 1));
                        }
                    });
                    ui.add_enabled_ui(i + 1 < n, |ui| {
                        if ui.button("↓").clicked() {
                            swap_pair = Some((i, i + 1));
                        }
                    });

                    // 삭제(기준이 2개 이상일 때만).
                    if n > 1 && ui.button("✖").clicked() {
                        remove_idx = Some(i);
                    }
                });
            }

            ui.separator();
            ui.horizontal(|ui| {
                // 아직 안 쓰인 컬럼이 있어야, 그리고 MAX_KEYS/전체 컬럼 수 미만일 때만
                // 기준을 추가할 수 있다(같은 컬럼 중복 금지 정책).
                let used: Vec<usize> = doc.sort_specs.iter().map(|s| s.col).collect();
                let next_free = (0..col_count).find(|c| !used.contains(c));
                let can_add = doc.sort_specs.len() < sort::MAX_KEYS
                    && doc.sort_specs.len() < col_count
                    && next_free.is_some();

                ui.add_enabled_ui(can_add, |ui| {
                    if ui.button("+ 기준 추가").clicked() {
                        if let Some(col) = next_free {
                            doc.sort_specs.push(SortSpec {
                                col,
                                kind: SortKind::Text,
                                dir: SortDir::Asc,
                                ci: true,
                            });
                        }
                    }
                });
                if doc.sort_specs.len() >= sort::MAX_KEYS {
                    ui.label(format!("(최대 {}개)", sort::MAX_KEYS));
                } else if doc.sort_specs.len() >= col_count {
                    ui.label("(모든 컬럼 사용 중)");
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
    if let Some((a, b)) = swap_pair {
        if a < doc.sort_specs.len() && b < doc.sort_specs.len() {
            doc.sort_specs.swap(a, b);
        }
    }
    // 창 X로 닫아도 다이얼로그 종료.
    if !open {
        doc.show_sort_dialog = false;
    }

    if do_sort && !doc.sort_specs.is_empty() {
        doc.show_sort_dialog = false;
        if doc.edit.is_some() {
            // 편집 모드: 백그라운드 permutation 대신 lines를 물리적으로 재배치한다.
            // 인메모리 정렬이라 동기 호출로 충분하다.
            let specs = doc.sort_specs.clone();
            apply_edit_sort(doc, &specs, delim, data_start);
        } else {
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
}

/// 편집 모드 정렬: `sort_lines`로 순서를 구해 `lines`를 실제로 재배치한다.
///
/// 뷰 모드의 permutation 정렬과 달리 결과가 버퍼에 그대로 반영되므로
/// `doc.sort`(SortState)는 **설정하지 않는다** — 유지할 permutation이 없고,
/// 헤더 화살표/상태바가 있지도 않은 라이브 정렬을 주장하면 안 된다. 정렬 뒤
/// 행을 삽입해도 재정렬되지 않고 삽입 위치에 그대로 남는다.
///
/// 셀 편집/선택 상태는 행이 움직이면 가리키는 대상이 달라지므로 초기화한다.
fn apply_edit_sort(doc: &mut Document, specs: &[SortSpec], delim: u8, data_start: usize) {
    let Some(e) = doc.edit.as_mut() else { return };
    if specs.is_empty() || e.lines.len() <= data_start {
        return;
    }
    let order = sort::sort_lines(&e.lines, specs, delim, data_start);
    crate::edit::apply_permutation(&mut e.lines, &order, data_start);
    e.dirty = true;
    // 정렬로 행이 뒤섞였으니 행을 가리키던 상태는 무효.
    doc.editing_cell = None;
    doc.cell_edit_text.clear();
    doc.cell_sel = None;
    doc.cell_drag_active = false;
    // 편집 모드 정렬은 permutation이 남지 않는다(이미 lines에 반영됨).
    doc.sort = None;
    doc.sort_job = None;
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

    // 편집 모드는 버퍼가 파일 전체를 이미 담고 있으므로 인덱싱 진행 상태와
    // 무관하게 정렬할 수 있다.
    let editing = doc.edit.is_some();
    let complete = editing || doc.index.status().phase == Phase::Complete;
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
                    ci: true,
                });
            }
            doc.show_sort_dialog = true;
        }
    });

    // 정렬 해제 버튼은 뷰 모드에서 permutation 정렬이 적용돼 있을 때만.
    // (편집 모드 정렬은 lines에 이미 반영돼 되돌릴 permutation이 없다.)
    if doc.sort.is_some() && ui.button("정렬 해제").clicked() {
        doc.sort = None;
    }

    if !complete && selected.is_some() {
        ui.label("(인덱싱 완료 후 정렬 가능)");
    }

    // 정렬 버튼이 눌리면 — 편집 모드면 lines를 즉시 재배치하고,
    // 뷰 모드면 백그라운드 permutation 작업을 띄운다.
    if let (Some((kind, dir)), Some(col)) = (do_sort, selected) {
        // 단일 문자 정렬은 대소문자 무시를 기본으로(사람 직관). 세밀 제어는
        // 다중 정렬 다이얼로그에서.
        let ci = kind == SortKind::Text;
        let spec = SortSpec { col, kind, dir, ci };
        if editing {
            apply_edit_sort(doc, &[spec], delim, data_start);
        } else {
            doc.sort_job = Some(sort::spawn_sort(
                doc.source.clone(),
                doc.index.clone(),
                doc.enc,
                delim,
                col,
                data_start,
                kind,
                dir,
                ci,
                ctx.clone(),
            ));
        }
    }
}

/// 우클릭 컨텍스트 메뉴에서 고른 동작. 클로저 안에서는 doc을 가변으로 빌릴 수
/// 없으므로 "무엇을 눌렀는지"만 기록해 두고, 테이블 클로저가 끝난 뒤 적용한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellMenuAction {
    Copy,
    Cut,
    Paste,
    Clear,
    InsertRowAbove,
    InsertRowBelow,
    DeleteRows,
}

/// 사각 선택을 정규화한다: (r0<=r1, c0<=c1).
fn normalize_rect(sel: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    let (r0, c0, r1, c1) = sel;
    (r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1))
}

/// (row, col)이 정규화 전 선택 사각형 안에 들어가는지.
fn rect_contains(sel: (usize, usize, usize, usize), row: usize, col: usize) -> bool {
    let (r0, c0, r1, c1) = normalize_rect(sel);
    (r0..=r1).contains(&row) && (c0..=c1).contains(&col)
}

/// 표 모드 렌더: 라인번호 + 구분자로 분리한 필드 컬럼들.
/// 헤더 클릭으로 컬럼을 선택하고, 정렬이 적용돼 있으면 permutation 순서로 렌더.
/// 편집 모드(`doc.edit.is_some()`)에서는 셀 편집/드래그 선택/우클릭 메뉴가 켜진다.
fn render_table(
    ui: &mut egui::Ui,
    doc: &mut Document,
    delim: u8,
    row_base: usize,
    col_base: usize,
    clipboard: &mut String,
) {
    use std::cell::{Cell, RefCell};

    // 헤더 행 데이터(있으면 첫 줄)와 데이터 시작 행 결정
    let total_lines = match &doc.edit {
        Some(e) => e.lines.len(),
        None => doc.index.line_count(),
    };
    let header_fields: Option<Vec<String>> = if doc.has_header && total_lines > 0 {
        parse_logical_line_edit(doc, 0, delim)
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

    // ---- 편집 모드 상태 스냅샷 + 클로저 → 바깥 인텐트 통로 ----
    // 테이블 클로저는 doc을 불변으로만 빌린다. 셀 상호작용 결과는 여기 모아
    // 두었다가 클로저가 끝난 뒤 doc.edit에 적용한다(기존 clicked_col과 동일 패턴).
    let editing = doc.edit.is_some();
    // cell_sel / editing_cell은 논리 행번호를 담는다(편집 모드에선 sort=None이므로
    // logical = data_start + view_row).
    let cur_sel = doc.cell_sel;
    let editing_cell = doc.editing_cell;
    // 편집 중 텍스트는 클로저 안에서 &mut로 써야 하므로 로컬 버퍼에 복사했다가
    // 클로저 종료 후 doc.cell_edit_text에 되돌린다(편집 중일 때만 복사).
    let edit_text: RefCell<String> = RefCell::new(if doc.editing_cell.is_some() {
        doc.cell_edit_text.clone()
    } else {
        String::new()
    });

    // 드래그 시작 셀(논리 행, 열). 드래그 중 확장 끝점.
    let drag_anchor: Cell<Option<(usize, usize)>> = Cell::new(None);
    let drag_head: Cell<Option<(usize, usize)>> = Cell::new(None);
    // 이번 프레임에 "셀 위에서" 좌클릭 누름이 진행 중인지. doc.cell_drag_active를
    // 클로저 밖에서 켜기 위한 통로(클로저는 doc을 불변으로만 빌린다).
    let cell_press: Cell<bool> = Cell::new(false);
    // 더블클릭으로 편집을 시작할 셀 + 그 셀의 현재 값.
    let begin_edit: RefCell<Option<(usize, usize, String)>> = RefCell::new(None);
    // 편집 중 셀의 커밋(Enter 또는 포커스 상실) 요청.
    let commit_edit: Cell<bool> = Cell::new(false);
    // Esc — 편집 취소(값 버림). 커밋보다 우선한다.
    let cancel_edit: Cell<bool> = Cell::new(false);
    // 우클릭으로 선택을 단일 셀로 바꿔야 하는 경우.
    let menu_target: Cell<Option<(usize, usize)>> = Cell::new(None);
    // 컨텍스트 메뉴에서 고른 동작.
    let menu_action: Cell<Option<CellMenuAction>> = Cell::new(None);
    // 좌클릭 버튼이 눌린 상태인지(드래그 확장 판정용). 프레임당 한 번만 읽는다.
    let primary_down = ui.input(|i| i.pointer.primary_down());
    // 이전 프레임까지 이어져 온 "셀에서 시작된 드래그" 상태. 버튼이 떼어진
    // 프레임부터는 무조건 꺼진 것으로 본다.
    let drag_active = doc.cell_drag_active && primary_down;

    // col_count는 헤더 필드 수와, 앞부분 데이터 행 몇 개를 샘플링한 필드 수의
    // 최댓값으로 정한다. 헤더가 없는 파일(header_fields == None)에서 1로
    // 고정되어 컬럼이 다 숨는 문제, 그리고 헤더보다 넓은 행이 잘리는 문제를
    // 함께 해결한다.
    const COL_COUNT_SAMPLE_ROWS: usize = 10;
    let mut col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(0);
    for logical in data_start..data_start + COL_COUNT_SAMPLE_ROWS {
        if let Some(fields) = parse_logical_line_edit(doc, logical, delim) {
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
                    .and_then(|l| parse_logical_line_edit(doc, l, delim))
                    .unwrap_or_default();
                for c in 0..col_count {
                    row.col(|ui| {
                        let cell_rect = ui.max_rect();
                        // 선택된 컬럼은 셀 배경에 밝은 파란 음영(줄무늬 위에 반투명).
                        if selected_col == Some(c) {
                            ui.painter().rect_filled(
                                cell_rect,
                                0.0,
                                sel_shade(),
                            );
                        }

                        // ---- 뷰 전용 모드: 기존 동작 그대로(라벨만) ----
                        if !editing {
                            ui.add(
                                egui::Label::new(fields.get(c).cloned().unwrap_or_default())
                                    .truncate(),
                            );
                            return;
                        }

                        // ---- 편집 모드 ----
                        // 편집 모드에선 permutation이 없으므로 logical은 항상 Some.
                        let Some(lrow) = logical else {
                            ui.add(egui::Label::new("").truncate());
                            return;
                        };

                        // 이 셀이 편집 중이면 라벨 대신 인라인 TextEdit.
                        if editing_cell == Some((lrow, c)) {
                            let mut buf = edit_text.borrow_mut();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut *buf)
                                    .desired_width(cell_rect.width())
                                    .id(ui.id().with(("celledit", lrow, c))),
                            );
                            // 진입 프레임에 포커스를 준다(이미 있으면 no-op).
                            if !resp.has_focus() && !resp.lost_focus() {
                                resp.request_focus();
                            }
                            // Esc = 취소(값 버림). egui는 Esc에 포커스만 해제하므로
                            // 그대로 두면 lost_focus()로 커밋돼 버린다. 키 입력을
                            // lost_focus() 판정과 독립적으로 먼저 읽어, 같은 프레임에
                            // 둘 다 켜지면 취소가 이기게 한다(적용 단계에서 처리).
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                cancel_edit.set(true);
                            }
                            // 커밋 조건은 "포커스 상실" 하나로 충분하다 —
                            // singleline TextEdit은 Enter를 누르면 포커스를 놓고,
                            // 다른 곳을 클릭해도 포커스를 놓는다.
                            if resp.lost_focus() {
                                commit_edit.set(true);
                            }
                            return;
                        }

                        // 선택 사각형 음영(컬럼 음영과 같은 색).
                        if let Some(sel) = cur_sel {
                            if rect_contains(sel, lrow, c) {
                                ui.painter().rect_filled(cell_rect, 0.0, sel_shade());
                            }
                        }

                        ui.add(
                            egui::Label::new(fields.get(c).cloned().unwrap_or_default())
                                .truncate(),
                        );

                        // 셀 전체를 클릭/드래그 대상으로. Label 뒤에 interact를 걸어
                        // 셀 칸 어디를 눌러도 반응하게 한다.
                        // sense는 `focusable: false`를 명시한 TABLE_CELL_SENSE다 —
                        // `Sense::click_and_drag()`는 focusable: true라 그려진 셀마다
                        // Tab 순회 대상이 된다(상세 이유는 그 상수 주석 참조).
                        // 이 id는 인라인 편집기 id(`("celledit", ..)`)와 다르고,
                        // 애초에 편집 중인 셀은 위에서 early return 하므로 같은
                        // 프레임에 둘이 공존하지 않는다.
                        let cell_id = ui.id().with(("cell", lrow, c));
                        let resp = ui.interact(cell_rect, cell_id, TABLE_CELL_SENSE);

                        // 이 셀 위에서 좌클릭 누름이 시작/진행 중이면 "셀에서
                        // 시작된 드래그"로 표시한다. is_pointer_button_down_on()
                        // 하나로 충분하다 — drag_started()가 참이면 이미
                        // is_pointer_button_down_on()도 참이므로(전자가 후자를
                        // 함의) 별도로 or할 필요가 없다.
                        // `&& primary_down`이 핵심이다: is_pointer_button_down_on()은
                        // 버튼을 가리지 않으므로(우클릭 press에도 true) 이 게이트가
                        // 없으면 우클릭이 다중 셀 선택을 단일 셀로 무너뜨린다 —
                        // 우클릭 press 프레임에 앵커가 잡혀 cell_sel이 (X,c,X,c)로
                        // 붕괴하고, context_menu()는 release 시점에야 열리므로 그때는
                        // 이미 선택이 무너진 뒤다.
                        let pressed_here = resp.is_pointer_button_down_on() && primary_down;
                        if pressed_here {
                            cell_press.set(true);
                        }
                        // 새 앵커 = "이번 프레임에 이 셀에서 누름이 시작됐다".
                        // drag_active(이전 프레임부터 이어져 온 드래그)가 아닐 때만
                        // 앵커를 잡는다 — 드래그가 이어지는 동안 시작 셀은 계속
                        // is_pointer_button_down_on()이 참이므로, 그것만 보면 매
                        // 프레임 앵커/끝점이 시작 셀로 되돌아가 확장이 깨진다.
                        // 누르는 첫 프레임부터 앵커를 잡아야(clicked()는 release
                        // 에서만 켜진다) 아래 확장 분기가 옛 선택을 끌고 가지 않는다.
                        if resp.clicked() || (pressed_here && !drag_active) {
                            drag_anchor.set(Some((lrow, c)));
                            drag_head.set(Some((lrow, c)));
                        }
                        // 드래그 중 끝점 확장. 드래그 이벤트는 최초로 눌린 셀만
                        // 받으므로(egui가 포인터를 그 위젯에 캡처), 다른 행/열로
                        // 넘어가는 확장은 "포인터가 이 셀 위 + 셀에서 시작된 드래그
                        // 진행 중"으로 감지한다. primary_down만 보면 표 밖에서 누른
                        // 채 들어온 포인터도 선택을 끌고 간다.
                        if resp.contains_pointer() && drag_active {
                            drag_head.set(Some((lrow, c)));
                        }
                        if resp.double_clicked() {
                            let val = fields.get(c).cloned().unwrap_or_default();
                            *begin_edit.borrow_mut() = Some((lrow, c, val));
                        }

                        // 우클릭 컨텍스트 메뉴.
                        // 어떤 셀에서 우클릭했는지는 **메뉴가 열리는 프레임에만**
                        // 기록한다. context_menu의 클로저는 메뉴가 떠 있는 매
                        // 프레임 다시 돌기 때문에, 그 안에서 set하면 menu_target이
                        // 메뉴 수명 내내 Some으로 남아 아래 5)의 "선택 밖이면 단일
                        // 선택으로" 재클램프가 매 프레임 재실행된다. 표 모드는
                        // 현재 메뉴가 열린 채 선택을 바꿀 키 경로가 없어 증상이
                        // 드러나지 않지만, 텍스트 모드에서 Ctrl+A로 실제 터졌던
                        // 것과 같은 구조다. secondary_clicked()는 메뉴를 여는
                        // release 프레임에만 참이므로(egui의 메뉴 생성 조건
                        // `hovered && secondary_clicked`, `menu.rs:429`와 일치)
                        // 여는 순간만 잡는다.
                        if resp.secondary_clicked() {
                            menu_target.set(Some((lrow, c)));
                        }
                        resp.context_menu(|ui| {
                            let pick = |ui: &mut egui::Ui, label: &str, act: CellMenuAction| {
                                if ui.button(label).clicked() {
                                    menu_action.set(Some(act));
                                    ui.close_menu();
                                }
                            };
                            pick(ui, "복사", CellMenuAction::Copy);
                            pick(ui, "잘라내기", CellMenuAction::Cut);
                            pick(ui, "붙여넣기", CellMenuAction::Paste);
                            pick(ui, "셀 내용 지우기", CellMenuAction::Clear);
                            ui.separator();
                            pick(ui, "위에 행 삽입", CellMenuAction::InsertRowAbove);
                            pick(ui, "아래에 행 삽입", CellMenuAction::InsertRowBelow);
                            pick(ui, "행 삭제", CellMenuAction::DeleteRows);
                        });

                        // `TABLE_CELL_SENSE`의 `focusable: false`만으로는 부족하다.
                        // `Response::context_menu`는 내부에서
                        // `response.interact(Sense::click())`을 부르는데
                        // (`egui-0.28.1/src/menu.rs:415`), `Sense::click()`은
                        // `focusable: true`라 sense가 **union**되어
                        // (`response.rs:855-868`) 이 셀이 다시 focusable 위젯으로
                        // 등록된다. 그래서 이 셀이 포커스를 쥐고 있으면 즉시 반납한다.
                        // (`surrender_focus`는 그 id가 포커스일 때만 지우므로
                        //  `memory.rs:762-767`, 인라인 셀 편집기의 TextEdit이나
                        //  툴바 위젯의 포커스는 건드리지 않는다 — 그것들은 id가
                        //  다르고, 편집 중인 셀은 이 코드에 도달하지도 않는다.)
                        ui.memory_mut(|m| m.surrender_focus(cell_id));
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

    // ---- 여기서부터 편집 인텐트 적용(테이블 클로저 종료 → doc 가변 대여 가능) ----
    if !editing {
        return;
    }
    // 편집 중 텍스트 버퍼를 doc으로 되돌린다(TextEdit이 로컬 버퍼를 고쳤을 수 있음).
    // 프레임 시작 시 편집 중이 아니었다면 로컬 버퍼는 빈 더미이므로 쓰지 않는다.
    if doc.editing_cell.is_some() {
        doc.cell_edit_text = edit_text.into_inner();
    }

    // 1) 셀 편집 종료. Esc(취소)가 커밋을 이긴다 — 값을 버리고 편집만 닫는다.
    if cancel_edit.get() {
        doc.editing_cell = None;
        doc.cell_edit_text.clear();
    } else if commit_edit.get() {
        // 커밋 — 새 편집 시작보다 먼저 처리해야 이전 셀 값이 저장된다.
        commit_editing_cell(doc, delim);
        doc.editing_cell = None;
        doc.cell_edit_text.clear();
    }

    // 2) 드래그 시작 지점 추적. 셀 위에서 누름이 감지되면 켜고, 버튼을 떼면 끈다.
    // 표 밖(툴바/빈 공간)에서 누른 채 표로 들어온 포인터는 끝내 켜지지 않는다.
    doc.cell_drag_active =
        next_cell_drag_active(doc.cell_drag_active, primary_down, cell_press.get());

    // 3) 드래그/클릭 선택 갱신. 앵커가 새로 잡히면 앵커+끝점을 함께 설정하고,
    // 이전 프레임에 시작된 드래그가 이어지는 중이면(셀에서 시작 + 버튼 눌림)
    // 앵커는 유지한 채 끝점만 확장한다.
    if let Some((ar, ac)) = drag_anchor.get() {
        let (hr, hc) = drag_head.get().unwrap_or((ar, ac));
        doc.cell_sel = Some((ar, ac, hr, hc));
    } else if drag_active {
        if let (Some(sel), Some((hr, hc))) = (doc.cell_sel, drag_head.get()) {
            doc.cell_sel = Some((sel.0, sel.1, hr, hc));
        }
    }

    // 4) 더블클릭 → 셀 편집 시작.
    if let Some((lrow, c, val)) = begin_edit.into_inner() {
        // 다른 셀이 아직 편집 중이면(예: 편집 셀이 스크롤 밖으로 나가 lost_focus를
        // 못 받은 경우) 그 값을 먼저 커밋해 편집 내용을 잃지 않게 한다.
        if doc.editing_cell.is_some() && doc.editing_cell != Some((lrow, c)) {
            commit_editing_cell(doc, delim);
        }
        doc.editing_cell = Some((lrow, c));
        doc.cell_edit_text = val;
        // 편집 시작 셀을 단일 선택으로.
        doc.cell_sel = Some((lrow, c, lrow, c));
    }

    // 5) 컨텍스트 메뉴 동작.
    // 우클릭 셀이 현재 선택 밖이면 그 셀을 단일 선택으로 만든다.
    // menu_target은 메뉴가 열리는 프레임에만 채워진다(위 우클릭 메뉴 참조).
    // 판정은 프레임 시작 스냅샷(`cur_sel`)이 아니라 **현재** `doc.cell_sel`로
    // 한다 — 위 3)/4)가 이미 선택을 바꿨을 수 있고, 그 최신 선택이 "우클릭이
    // 선택 안이었나"의 진실이다.
    if let Some((lrow, c)) = menu_target.get() {
        let inside = doc.cell_sel.map_or(false, |s| rect_contains(s, lrow, c));
        if !inside {
            doc.cell_sel = Some((lrow, c, lrow, c));
        }
    }
    if let Some(act) = menu_action.get() {
        apply_cell_menu_action(ui, doc, delim, clipboard, act);
    }
}

/// "셀에서 시작된 드래그가 진행 중인가" 상태 전이. egui의 `primary_down`은
/// 전역 버튼 상태라 그것만으로는 누름이 표 안에서 시작됐는지 알 수 없으므로,
/// 셀 위 누름(`pressed_on_cell`)이 관측된 적이 있는지를 별도로 래치한다.
///
/// - 버튼을 떼면(`primary_down == false`) 무조건 꺼진다.
/// - 셀 위에서 누름이 관측되면 켜진다.
/// - 그 외에는 이전 상태를 유지한다(드래그가 셀 밖으로 나갔다 돌아와도 유지).
fn next_cell_drag_active(prev: bool, primary_down: bool, pressed_on_cell: bool) -> bool {
    if !primary_down {
        return false;
    }
    prev || pressed_on_cell
}

/// 현재 `editing_cell`의 `cell_edit_text`를 편집 버퍼에 써넣는다(dirty 표시).
/// `editing_cell` 자체는 지우지 않는다 — 호출측이 상황에 맞게 정리한다.
fn commit_editing_cell(doc: &mut Document, delim: u8) {
    let Some((lrow, c)) = doc.editing_cell else { return };
    let text = doc.cell_edit_text.clone();
    let Some(e) = doc.edit.as_mut() else { return };
    if lrow >= e.lines.len() {
        return;
    }
    crate::edit::set_cell(&mut e.lines, lrow, c, &text, delim);
    e.dirty = true;
}

/// 컨텍스트 메뉴 동작을 편집 버퍼에 적용한다. 선택 사각형은 논리 행/열 기준.
fn apply_cell_menu_action(
    ui: &mut egui::Ui,
    doc: &mut Document,
    delim: u8,
    clipboard: &mut String,
    act: CellMenuAction,
) {
    let Some(sel) = doc.cell_sel else { return };
    let (r0, c0, r1, c1) = normalize_rect(sel);
    let Some(e) = doc.edit.as_mut() else { return };

    match act {
        CellMenuAction::Copy => {
            let tsv = crate::edit::cells_to_tsv(&e.lines, r0, c0, r1, c1, delim);
            *clipboard = tsv.clone();
            ui.output_mut(|o| o.copied_text = tsv);
        }
        CellMenuAction::Cut => {
            let tsv = crate::edit::cells_to_tsv(&e.lines, r0, c0, r1, c1, delim);
            *clipboard = tsv.clone();
            ui.output_mut(|o| o.copied_text = tsv);
            crate::edit::clear_cells(&mut e.lines, r0, c0, r1, c1, delim);
            e.dirty = true;
        }
        CellMenuAction::Paste => {
            if !clipboard.is_empty() {
                crate::edit::paste_tsv(&mut e.lines, r0, c0, clipboard, delim);
                e.dirty = true;
            }
        }
        CellMenuAction::Clear => {
            crate::edit::clear_cells(&mut e.lines, r0, c0, r1, c1, delim);
            e.dirty = true;
        }
        CellMenuAction::InsertRowAbove => {
            crate::edit::insert_row(&mut e.lines, r0, String::new());
            e.dirty = true;
            // 삽입된 빈 행 아래로 선택이 밀린다.
            doc.cell_sel = Some((r0 + 1, c0, r1 + 1, c1));
        }
        CellMenuAction::InsertRowBelow => {
            crate::edit::insert_row(&mut e.lines, r1 + 1, String::new());
            e.dirty = true;
        }
        CellMenuAction::DeleteRows => {
            // 뒤에서부터 지워야 인덱스가 밀리지 않는다.
            for r in (r0..=r1).rev() {
                crate::edit::remove_row(&mut e.lines, r);
            }
            e.dirty = true;
            // 선택은 삭제 지점 한 셀로 축소(범위 밖이면 마지막 행으로 클램프).
            let last = e.lines.len().saturating_sub(1);
            let r = r0.min(last);
            doc.cell_sel = Some((r, c0, r, c1));
            doc.editing_cell = None;
        }
    }
}

/// 텍스트 모드 우클릭 컨텍스트 메뉴 동작. `CellMenuAction`과 같은 이유로
/// (클로저 안에서 doc을 가변 대여할 수 없음) 인텐트만 기록해 두고 나중에 적용한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMenuAction {
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

/// 텍스트 모드 캐럿/선택 이동·편집 인텐트. 키 입력 루프가 이걸 만들어 내고,
/// 적용 단계에서 `doc.edit`에 반영한다.
enum TextEditIntent {
    /// 문자 입력(개행 포함 가능). 선택이 있으면 먼저 지운다.
    Insert(String),
    /// Enter — 선택 삭제 후 줄 나누기.
    Newline,
    Backspace,
    Delete,
    /// 캐럿 이동. `extend`면 앵커를 유지한 채 선택을 확장한다.
    Move(CaretMove, bool),
    SelectAll,
    Copy,
    Cut,
    Paste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaretMove {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// 텍스트 모드에서 쓰는 고정폭 폰트. 캐럿/선택 x 좌표 매핑을 위해 줄 텍스트를
/// 직접 레이아웃하므로, 렌더와 매핑이 같은 FontId를 써야 한다.
fn text_font_id() -> egui::FontId {
    egui::FontId::monospace(13.0)
}

/// 줄의 char 개수.
fn line_char_len(s: &str) -> usize {
    s.chars().count()
}

/// 캐럿을 문서 범위 안으로 클램프한다(줄 번호, 줄 안 col 모두).
fn clamp_pos(lines: &[String], p: crate::edit::TextPos) -> crate::edit::TextPos {
    if lines.is_empty() {
        return crate::edit::TextPos { line: 0, col: 0 };
    }
    let line = p.line.min(lines.len() - 1);
    let col = p.col.min(line_char_len(&lines[line]));
    crate::edit::TextPos { line, col }
}

/// 캐럿 이동 한 번을 적용한 새 위치. 좌/우는 줄 경계를 넘어간다.
fn apply_caret_move(
    lines: &[String],
    p: crate::edit::TextPos,
    mv: CaretMove,
) -> crate::edit::TextPos {
    use crate::edit::TextPos;
    let p = clamp_pos(lines, p);
    match mv {
        CaretMove::Left => {
            if p.col > 0 {
                TextPos { line: p.line, col: p.col - 1 }
            } else if p.line > 0 {
                TextPos { line: p.line - 1, col: line_char_len(&lines[p.line - 1]) }
            } else {
                p
            }
        }
        CaretMove::Right => {
            if p.col < line_char_len(&lines[p.line]) {
                TextPos { line: p.line, col: p.col + 1 }
            } else if p.line + 1 < lines.len() {
                TextPos { line: p.line + 1, col: 0 }
            } else {
                p
            }
        }
        // 위/아래는 col을 유지하되 대상 줄 길이로 클램프(일반 에디터 동작).
        CaretMove::Up => {
            if p.line == 0 {
                TextPos { line: 0, col: 0 }
            } else {
                clamp_pos(lines, TextPos { line: p.line - 1, col: p.col })
            }
        }
        CaretMove::Down => {
            if p.line + 1 >= lines.len() {
                TextPos { line: p.line, col: line_char_len(&lines[p.line]) }
            } else {
                clamp_pos(lines, TextPos { line: p.line + 1, col: p.col })
            }
        }
        CaretMove::Home => TextPos { line: p.line, col: 0 },
        CaretMove::End => TextPos { line: p.line, col: line_char_len(&lines[p.line]) },
    }
}

/// 방향키/Home/End 한 번의 결과 (새 캐럿, 새 선택).
///
/// - `extend`(Shift 동반)면 앵커를 유지한 채 캐럿만 옮겨 선택을 넓힌다.
///   선택이 없었다면 현재 캐럿이 앵커가 된다.
/// - `extend`가 아니고 선택이 있으면 좌/위는 선택 시작으로, 우/아래는 선택
///   끝으로 캐럿을 붕괴시킨다(일반 에디터 동작 — 첫 방향키는 선택 해제만).
///   Home/End는 붕괴 후 그 줄 안에서 이동한다.
/// - 선택이 없으면 그냥 한 칸 이동.
fn next_caret_and_sel(
    lines: &[String],
    caret: crate::edit::TextPos,
    sel: Option<(crate::edit::TextPos, crate::edit::TextPos)>,
    mv: CaretMove,
    extend: bool,
) -> (
    crate::edit::TextPos,
    Option<(crate::edit::TextPos, crate::edit::TextPos)>,
) {
    if extend {
        let anchor = sel.map(|(a, _)| a).unwrap_or(caret);
        let to = apply_caret_move(lines, caret, mv);
        let new_sel = if anchor == to { None } else { Some((anchor, to)) };
        return (to, new_sel);
    }
    match sel {
        Some((a, b)) => {
            let (lo, hi) = crate::edit::normalize(a, b);
            match mv {
                // 좌/위: 선택 시작으로 붕괴(이동 없음).
                CaretMove::Left | CaretMove::Up => (lo, None),
                // 우/아래: 선택 끝으로 붕괴(이동 없음).
                CaretMove::Right | CaretMove::Down => (hi, None),
                // Home/End: 캐럿이 있던 줄 안에서 이동.
                _ => (apply_caret_move(lines, caret, mv), None),
            }
        }
        None => (apply_caret_move(lines, caret, mv), None),
    }
}

/// Backspace/Delete가 버퍼를 실제로 바꿨는지 판정한다(dirty 표시 여부).
///
/// - 선택이 있었으면 그 범위를 지웠으므로 무조건 변경이다.
/// - 선택이 없었으면 캐럿이 움직였는지로 본다. Backspace는 한 글자/개행을
///   지울 때 반드시 캐럿이 뒤로 가고, 지울 게 없는 문서 맨 앞(0,0)에서는
///   `edit::backspace`가 위치를 그대로 돌려주므로 이 판정이 정확하다.
///
/// (Delete는 실제 삭제를 해도 캐럿이 제자리라 이 함수를 쓰지 않는다 —
///  호출측이 "삭제를 수행했는가"를 직접 본다.)
fn backspace_or_delete_changed(
    had_sel: bool,
    before: crate::edit::TextPos,
    after: crate::edit::TextPos,
) -> bool {
    had_sel || before != after
}

/// 문서 전체를 덮는 선택 (0,0) ~ (마지막 줄 끝).
fn whole_document_sel(lines: &[String]) -> (crate::edit::TextPos, crate::edit::TextPos) {
    use crate::edit::TextPos;
    let last = lines.len().saturating_sub(1);
    (
        TextPos { line: 0, col: 0 },
        TextPos { line: last, col: lines.get(last).map_or(0, |l| line_char_len(l)) },
    )
}

/// 정규화된 선택이 `line` 줄에서 덮는 char 구간 [c0, c1). 걸치지 않으면 None.
/// 줄 끝을 넘어가는 선택(다음 줄로 이어지는 경우)은 줄 길이 + 1로 표시해
/// "개행까지 선택됨"을 음영 폭으로 드러낸다.
fn sel_span_on_line(
    a: crate::edit::TextPos,
    b: crate::edit::TextPos,
    line: usize,
    len: usize,
) -> Option<(usize, usize)> {
    if line < a.line || line > b.line {
        return None;
    }
    let c0 = if line == a.line { a.col.min(len) } else { 0 };
    // 마지막 줄이 아니면 줄 끝 + 개행 한 칸까지.
    let c1 = if line == b.line { b.col.min(len) } else { len + 1 };
    if c0 >= c1 {
        return None;
    }
    Some((c0, c1))
}

/// 편집 모드 텍스트 줄이 쓰는 상호작용 sense.
///
/// `Sense::click_and_drag()`와 click/drag는 같지만 `focusable`이 다르다:
/// 그 생성자는 `focusable: true`를 넣는다(`egui-0.28.1/src/sense.rs:92-98`).
/// 본문 줄이 focusable이면 `Context::create_widget`이
/// `interested_in_focus`로 등록해(`context.rs:1050`) Tab 순회 대상이 되고,
/// Tab 한 번으로 포커스가 줄에 걸리면 `render_text`의 keyboard_free 게이트가
/// 닫혀 모든 키 입력이 삼켜진다. 그래서 명시적으로 opt-out 한다.
const TEXT_LINE_SENSE: egui::Sense = egui::Sense {
    click: true,
    drag: true,
    focusable: false,
};

/// 표 모드 데이터 셀이 쓰는 상호작용 sense. `TEXT_LINE_SENSE`와 값은 같지만
/// 이유가 달라 따로 둔다(한쪽을 고칠 때 다른 쪽이 딸려가지 않게).
///
/// 셀도 텍스트 줄과 똑같이 `Sense::click_and_drag()`(= `focusable: true`,
/// `egui-0.28.1/src/sense.rs:92-98`)를 쓰고 있었다. 그러면 화면에 그려진
/// **모든 셀**이 `interested_in_focus`로 등록돼(`context.rs:1050`) Tab 순회
/// 대상이 된다. 표 모드에는 `render_text`의 `keyboard_free` 같은 전역 키
/// 게이트가 없어 오늘 당장 편집 불가가 되지는 않지만,
///
/// - Tab이 셀 위에 눈에 보이지 않는 포커스를 남겨 인라인 셀 편집기
///   (`("celledit", ..)` TextEdit)의 포커스 이동과 간섭할 여지가 있고,
/// - 표 모드가 나중에 키보드 게이트를 갖는 순간 텍스트 모드와 똑같이
///   문서 전체가 편집 불가가 된다.
///
/// 셀은 포커스가 필요 없다 — 클릭/드래그는 `interact_pointer_pos` 경로로,
/// 편집은 별도의 `TextEdit` 위젯으로 처리한다. 그래서 명시적으로 opt-out 한다.
const TABLE_CELL_SENSE: egui::Sense = egui::Sense {
    click: true,
    drag: true,
    focusable: false,
};

/// 텍스트 모드 렌더: 라인번호 + 줄 전체(구분 안 함).
///
/// 뷰 전용 모드(`doc.edit == None`)에서는 기존과 동일하게 `Label` + truncate로
/// 그린다. 편집 모드에서는 캐럿/선택을 정확히 그려야 하므로 줄 텍스트를 직접
/// 고정폭 galley로 레이아웃해 그리고, char↔x 매핑에 그 galley의
/// `pos_from_ccursor` / `cursor_from_pos`를 쓴다(근사 아님).
fn render_text(ui: &mut egui::Ui, doc: &mut Document, row_base: usize, clipboard: &mut String) {
    use std::cell::Cell;

    let editing = doc.edit.is_some();
    let total_lines = match &doc.edit {
        Some(e) => e.lines.len(),
        None => doc.index.line_count(),
    };
    let avail_height = ui.available_height();

    // ---- 편집 모드 상태 스냅샷 + 클로저 → 바깥 인텐트 통로 ----
    // 표 모드와 같은 규율: 테이블 클로저는 doc을 불변으로만 빌리고, 상호작용
    // 결과는 여기 모아 두었다가 클로저 종료 후 doc.edit에 적용한다.
    let caret = doc.text_caret;
    // 정규화한 선택(음영 그리기용).
    let sel_norm = doc
        .text_sel
        .map(|(a, b)| crate::edit::normalize(a, b))
        .filter(|(a, b)| a != b);
    let font_id = text_font_id();
    let text_color = ui.visuals().text_color();
    let caret_color = ui.visuals().strong_text_color();

    // 클릭/드래그로 잡은 위치. anchor는 누름 시작, head는 확장 끝점.
    let drag_anchor: Cell<Option<crate::edit::TextPos>> = Cell::new(None);
    let drag_head: Cell<Option<crate::edit::TextPos>> = Cell::new(None);
    // 이번 프레임에 "텍스트 줄 위에서" 좌클릭 누름이 진행 중인지.
    let line_press: Cell<bool> = Cell::new(false);
    // 우클릭 대상 줄 위치 + 고른 메뉴 동작.
    let menu_target: Cell<Option<crate::edit::TextPos>> = Cell::new(None);
    let menu_action: Cell<Option<TextMenuAction>> = Cell::new(None);

    let primary_down = ui.input(|i| i.pointer.primary_down());
    // 이전 프레임부터 이어져 온 "줄에서 시작된 드래그". 버튼을 떼면 꺼진다.
    // (`next_cell_drag_active`와 같은 전이 규칙을 그대로 쓴다 — 표/텍스트 모드가
    // 다른 것은 무엇을 눌렀는지뿐이고, 래치 논리는 동일하다.)
    let drag_active = doc.text_drag_active && primary_down;

    // 키 입력은 프레임당 한 번만 읽는다. 편집 모드 + 다른 위젯이 키보드 포커스를
    // 갖고 있지 않을 때만 소비한다 — 그렇지 않으면 툴바의 "직접:" 구분자
    // TextEdit 등에 타이핑할 때 같은 키가 본문에도 들어간다.
    //
    // 이 게이트가 성립하는 전제는 "본문 텍스트 줄이 포커스를 가져갈 수 없다"이다.
    // 그 전제는 저절로 성립하지 않는다 — `Sense::click_and_drag()`는
    // `focusable: true`이고(`egui-0.28.1/src/sense.rs:92-98`), 그런 sense로 만든
    // 위젯은 `context.rs:1050`에서 `interested_in_focus`로 등록되어 Tab 순회
    // 대상이 된다. 그래서 아래 줄 interact는 `focusable: false`를 **명시적으로**
    // 지정한다(TEXT_LINE_SENSE). 그 결과 Tab이 본문 줄로 포커스를 옮길 수 없고,
    // "포커스 없음" = "본문이 입력을 받는다"가 실제로 참이 된다.
    // (명시하지 않으면 Tab 한 번으로 포커스가 줄에 걸려 모든 키 입력이 조용히
    //  삼켜지고 문서가 편집 불가 상태가 된다.)
    let intents: Vec<TextEditIntent> = if editing {
        let keyboard_free = ui.memory(|m| m.focused().is_none());
        if keyboard_free {
            ui.input(collect_text_intents)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

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
                let line = logical_line(doc, logical).unwrap_or_default();
                row.col(|ui| {
                    // ---- 뷰 전용 모드: 기존 동작 그대로(라벨만) ----
                    if !editing {
                        ui.add(egui::Label::new(line).truncate());
                        return;
                    }

                    // ---- 편집 모드 ----
                    let cell_rect = ui.max_rect();
                    let len = line_char_len(&line);
                    // 줄 텍스트를 고정폭으로 직접 레이아웃한다. 이 galley 하나가
                    // (a) 실제 그리는 글자, (b) char→x, (c) x→char의 유일한
                    // 진실이므로 캐럿/음영이 글자와 어긋날 수 없다.
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(line.clone(), font_id.clone(), text_color)
                    });
                    // 셀 왼쪽 위에 세로 중앙 정렬해 그린다.
                    let origin = egui::pos2(
                        cell_rect.left(),
                        cell_rect.center().y - galley.size().y * 0.5,
                    );
                    // char 인덱스 → 셀 좌표 x.
                    let x_of = |c: usize| -> f32 {
                        origin.x
                            + galley
                                .pos_from_ccursor(egui::text::CCursor::new(c.min(len)))
                                .min
                                .x
                    };

                    let painter = ui.painter().with_clip_rect(cell_rect);
                    // 1) 선택 음영을 글자 아래에 먼저.
                    if let Some((a, b)) = sel_norm {
                        if let Some((c0, c1)) = sel_span_on_line(a, b, logical, len) {
                            // c1이 len+1이면(개행 포함) 줄 끝 너머 한 칸을 더 칠한다.
                            let x0 = x_of(c0);
                            let x1 = if c1 > len {
                                x_of(len) + galley.size().y * 0.4
                            } else {
                                x_of(c1)
                            };
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(x0, cell_rect.top()),
                                    egui::pos2(x1, cell_rect.bottom()),
                                ),
                                0.0,
                                sel_shade(),
                            );
                        }
                    }
                    // 2) 글자.
                    painter.galley(origin, galley.clone(), text_color);
                    // 3) 캐럿(그 줄일 때만).
                    if caret.line == logical {
                        let x = x_of(caret.col);
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(x, cell_rect.top() + 2.0),
                                egui::pos2(x + 1.5, cell_rect.bottom() - 2.0),
                            ),
                            0.0,
                            caret_color,
                        );
                    }

                    // 4) 클릭/드래그 상호작용. 셀 전체가 대상.
                    // sense를 직접 만들어 `focusable: false`를 강제한다 —
                    // `Sense::click_and_drag()`는 focusable: true라 Tab이 여기로
                    // 포커스를 옮길 수 있고, 그러면 위쪽 keyboard_free 게이트가
                    // 닫혀 문서 전체가 편집 불가가 된다(위 주석 참조).
                    let id = ui.id().with(("textline", logical));
                    let resp = ui.interact(cell_rect, id, TEXT_LINE_SENSE);
                    // 포인터 x → char 인덱스(같은 galley로 역매핑).
                    let pos_at_pointer = |pp: egui::Pos2| -> crate::edit::TextPos {
                        let local = pp - origin;
                        // y는 한 줄짜리 galley이므로 0으로 눌러 첫 행에 붙인다.
                        let cur = galley.cursor_from_pos(egui::vec2(local.x, 0.0));
                        crate::edit::TextPos {
                            line: logical,
                            col: cur.ccursor.index.min(len),
                        }
                    };

                    // `&& primary_down` 게이트는 표 모드와 같은 이유다:
                    // is_pointer_button_down_on()은 버튼을 가리지 않아 우클릭
                    // press에도 참이므로, 게이트가 없으면 우클릭이 선택을
                    // 캐럿 하나로 무너뜨린다(메뉴는 release에서야 열린다).
                    let pressed_here = resp.is_pointer_button_down_on() && primary_down;
                    if pressed_here {
                        line_press.set(true);
                    }
                    if let Some(pp) = resp.interact_pointer_pos() {
                        let p = pos_at_pointer(pp);
                        // 새 앵커 = 이번 프레임에 이 줄에서 누름이 시작됐다.
                        // 드래그가 이어지는 동안(drag_active)에는 앵커를 다시
                        // 잡지 않아야 확장이 깨지지 않는다.
                        if resp.clicked() || (pressed_here && !drag_active) {
                            drag_anchor.set(Some(p));
                            drag_head.set(Some(p));
                        }
                    }
                    // 드래그 중 확장: 포인터가 이 줄 위 + 줄에서 시작된 드래그.
                    // 다른 줄로 넘어가는 확장은 원 위젯이 포인터를 캡처하므로
                    // contains_pointer()로 감지한다(표 모드와 동일).
                    if drag_active && resp.contains_pointer() {
                        if let Some(pp) = ui.input(|i| i.pointer.latest_pos()) {
                            drag_head.set(Some(pos_at_pointer(pp)));
                        }
                    }

                    // 5) 우클릭 컨텍스트 메뉴.
                    // 어떤 줄에서 우클릭했는지는 **메뉴가 열리는 프레임에만** 기록한다.
                    // context_menu의 클로저는 메뉴가 떠 있는 매 프레임 다시 돌기
                    // 때문에, 그 안에서 set하면 menu_target이 메뉴 수명 내내
                    // Some으로 남아 아래 4)의 "선택 밖이면 선택 해제" 판정이 매
                    // 프레임 재실행된다. 그러면 메뉴가 열린 채 Ctrl+A/Shift+화살표로
                    // 만든 **새 선택이 곧바로 지워진다**. secondary_clicked()는
                    // 메뉴를 여는 release 프레임에만 참이므로 여는 순간만 잡는다.
                    // col은 줄 끝으로 둔다 — 메뉴가 열린 뒤 포인터는 메뉴 창 위에
                    // 있어 이 줄 좌표계로 되돌릴 수 없고, 메뉴 동작들은
                    // "선택 밖이면 그 줄로" 이상의 정밀도를 쓰지 않는다.
                    if resp.secondary_clicked() {
                        menu_target.set(Some(crate::edit::TextPos {
                            line: logical,
                            col: len,
                        }));
                    }
                    resp.context_menu(|ui| {
                        let pick = |ui: &mut egui::Ui, label: &str, act: TextMenuAction| {
                            if ui.button(label).clicked() {
                                menu_action.set(Some(act));
                                ui.close_menu();
                            }
                        };
                        pick(ui, "잘라내기", TextMenuAction::Cut);
                        pick(ui, "복사", TextMenuAction::Copy);
                        pick(ui, "붙여넣기", TextMenuAction::Paste);
                        pick(ui, "삭제", TextMenuAction::Delete);
                        ui.separator();
                        pick(ui, "전체 선택", TextMenuAction::SelectAll);
                    });

                    // `TEXT_LINE_SENSE`의 `focusable: false`만으로는 부족하다.
                    // `Response::context_menu`는 내부에서
                    // `response.interact(Sense::click())`을 부르는데
                    // (`egui-0.28.1/src/menu.rs:415`), `Sense::click()`은
                    // `focusable: true`라 sense가 **union**되어
                    // (`response.rs:855-868`) 이 줄이 다시 focusable 위젯으로
                    // 등록된다. 그러면 Tab이 또 줄에 걸려 keyboard_free 게이트가
                    // 닫힌다. 그래서 이 줄이 포커스를 쥐고 있으면 즉시 반납한다.
                    // (`surrender_focus`는 그 id가 포커스일 때만 지우므로
                    //  툴바 TextEdit 등 다른 위젯의 포커스는 건드리지 않는다.)
                    ui.memory_mut(|m| m.surrender_focus(id));
                });
            });
        });

    // ---- 클로저 종료 → doc 가변 대여 가능 ----
    if !editing {
        return;
    }

    // 1) 드래그 원점 래치 갱신(표 모드와 같은 전이 규칙).
    doc.text_drag_active =
        next_cell_drag_active(doc.text_drag_active, primary_down, line_press.get());

    // 2) 마우스 선택/캐럿 갱신.
    if let Some(anchor) = drag_anchor.get() {
        let head = drag_head.get().unwrap_or(anchor);
        doc.text_caret = head;
        doc.text_sel = if anchor == head { None } else { Some((anchor, head)) };
    } else if drag_active {
        if let Some(head) = drag_head.get() {
            // 앵커는 유지하고 끝점만 확장. 선택이 없었으면 이전 캐럿이 앵커.
            let anchor = doc.text_sel.map(|(a, _)| a).unwrap_or(doc.text_caret);
            doc.text_caret = head;
            doc.text_sel = if anchor == head { None } else { Some((anchor, head)) };
        }
    }

    // 3) 키 입력 인텐트 적용.
    for intent in intents {
        apply_text_intent(ui, doc, clipboard, intent);
    }

    // 4) 컨텍스트 메뉴 동작. 우클릭 줄이 현재 선택 밖이면 캐럿만 그 줄로 옮긴다
    //    (선택은 유지 — 선택 안에서 우클릭한 경우 그 선택에 대해 동작해야 한다).
    //    menu_target은 메뉴가 열리는 프레임에만 채워진다(위 5) 참조).
    //    판정은 프레임 시작 스냅샷(sel_norm)이 아니라 **현재** doc.text_sel로
    //    한다 — 3)의 키 인텐트가 이미 선택을 바꿨을 수 있고, 그 최신 선택이
    //    "우클릭이 선택 안이었나"의 진실이다.
    if let Some(t) = menu_target.get() {
        let cur_sel = doc
            .text_sel
            .map(|(a, b)| crate::edit::normalize(a, b))
            .filter(|(a, b)| a != b);
        let inside = cur_sel.map_or(false, |(a, b)| {
            (a.line..=b.line).contains(&t.line)
        });
        if !inside {
            doc.text_sel = None;
            if let Some(e) = &doc.edit {
                doc.text_caret = clamp_pos(&e.lines, t);
            }
        }
    }
    if let Some(act) = menu_action.get() {
        let intent = match act {
            TextMenuAction::Cut => TextEditIntent::Cut,
            TextMenuAction::Copy => TextEditIntent::Copy,
            TextMenuAction::Paste => TextEditIntent::Paste(clipboard.clone()),
            TextMenuAction::Delete => TextEditIntent::Delete,
            TextMenuAction::SelectAll => TextEditIntent::SelectAll,
        };
        apply_text_intent(ui, doc, clipboard, intent);
    }

    // 5) 프레임 마무리: 캐럿/선택을 현재 버퍼 범위로 클램프해 다음 프레임 렌더와
    //    다음 인텐트 적용이 항상 유효한 위치에서 시작하게 한다.
    if let Some(e) = &doc.edit {
        doc.text_caret = clamp_pos(&e.lines, doc.text_caret);
        doc.text_sel = doc
            .text_sel
            .map(|(a, b)| (clamp_pos(&e.lines, a), clamp_pos(&e.lines, b)))
            .filter(|(a, b)| a != b);
    }
}

/// 이번 프레임의 입력 이벤트에서 텍스트 편집 인텐트를 뽑는다.
/// egui-winit은 Ctrl+C/X/V를 `Event::Copy`/`Cut`/`Paste`로 변환해 보내고
/// `Key` 이벤트는 만들지 않으므로, 그 세 개는 이벤트로만 처리한다.
/// `Event::Text`는 ctrl/command가 눌린 동안에는 오지 않는다(같은 이유).
fn collect_text_intents(i: &egui::InputState) -> Vec<TextEditIntent> {
    let mut out = Vec::new();
    for ev in &i.events {
        match ev {
            egui::Event::Text(t) if !t.is_empty() => {
                out.push(TextEditIntent::Insert(t.clone()));
            }
            egui::Event::Copy => out.push(TextEditIntent::Copy),
            egui::Event::Cut => out.push(TextEditIntent::Cut),
            egui::Event::Paste(s) => out.push(TextEditIntent::Paste(s.clone())),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let shift = modifiers.shift;
                let ctrl = modifiers.ctrl || modifiers.command;
                match key {
                    egui::Key::Enter => out.push(TextEditIntent::Newline),
                    egui::Key::Backspace => out.push(TextEditIntent::Backspace),
                    egui::Key::Delete => out.push(TextEditIntent::Delete),
                    egui::Key::A if ctrl => out.push(TextEditIntent::SelectAll),
                    egui::Key::ArrowLeft => {
                        out.push(TextEditIntent::Move(CaretMove::Left, shift))
                    }
                    egui::Key::ArrowRight => {
                        out.push(TextEditIntent::Move(CaretMove::Right, shift))
                    }
                    egui::Key::ArrowUp => out.push(TextEditIntent::Move(CaretMove::Up, shift)),
                    egui::Key::ArrowDown => {
                        out.push(TextEditIntent::Move(CaretMove::Down, shift))
                    }
                    egui::Key::Home => out.push(TextEditIntent::Move(CaretMove::Home, shift)),
                    egui::Key::End => out.push(TextEditIntent::Move(CaretMove::End, shift)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// 인텐트 하나를 편집 버퍼에 적용한다. 모든 변경은 `dirty = true`.
///
/// 핵심 불변식: `lines[i]`에는 `\n`/`\r`가 들어가면 안 된다. 그래서
/// 문자 삽입은 항상 `insert_str`(개행을 새 줄로 분해)을 거치고, Enter는
/// `split_line`으로 라우팅한다 — `insert_char`에 `'\n'`을 넘기지 않는다.
fn apply_text_intent(
    ui: &mut egui::Ui,
    doc: &mut Document,
    clipboard: &mut String,
    intent: TextEditIntent,
) {
    use crate::edit::{backspace, delete_range, insert_str, normalize, selection_text, split_line};

    let caret = doc.text_caret;
    let sel_raw = doc.text_sel;
    let Some(e) = doc.edit.as_mut() else { return };
    if e.lines.is_empty() {
        e.lines.push(String::new());
    }
    // 캐럿/선택은 반드시 현재 lines 범위로 클램프한 뒤 쓴다. delete_range 등은
    // lines[pos.line]을 직접 인덱싱하므로, 한 프레임에 여러 인텐트가 연속으로
    // 적용돼 줄 수가 줄어든 뒤의 오래된 위치를 그대로 넘기면 패닉한다.
    let caret = clamp_pos(&e.lines, caret);
    let sel = sel_raw
        .map(|(a, b)| (clamp_pos(&e.lines, a), clamp_pos(&e.lines, b)))
        .filter(|(a, b)| a != b);

    // 선택을 먼저 지우고 그 지점을 새 캐럿으로 삼는 공통 처리.
    let delete_sel = |lines: &mut Vec<String>, sel: Option<(_, _)>| -> Option<crate::edit::TextPos> {
        sel.map(|(a, b)| delete_range(lines, a, b))
    };

    match intent {
        TextEditIntent::Insert(t) => {
            // \r은 버리고 \n만 남긴다 — insert_str이 \n을 줄 분할로 처리한다.
            let t = t.replace('\r', "");
            if t.is_empty() {
                return;
            }
            let at = delete_sel(&mut e.lines, sel).unwrap_or(caret);
            doc.text_caret = insert_str(&mut e.lines, at, &t);
            doc.text_sel = None;
            e.dirty = true;
        }
        TextEditIntent::Newline => {
            let at = delete_sel(&mut e.lines, sel).unwrap_or(caret);
            doc.text_caret = split_line(&mut e.lines, at);
            doc.text_sel = None;
            e.dirty = true;
        }
        TextEditIntent::Backspace => {
            // 아무것도 지우지 않았으면 dirty를 세우지 않는다. 선택이 없고
            // 캐럿이 문서 맨 앞(0,0)이면 backspace는 위치를 그대로 돌려주는
            // no-op이다 — 그때도 dirty를 세우면 파일을 열자마자 Backspace 한 번에
            // "저장 안 됨" 상태가 생긴다.
            let had_sel = sel.is_some();
            doc.text_caret = match delete_sel(&mut e.lines, sel) {
                Some(p) => p,
                None => backspace(&mut e.lines, caret),
            };
            doc.text_sel = None;
            if backspace_or_delete_changed(had_sel, caret, doc.text_caret) {
                e.dirty = true;
            }
        }
        TextEditIntent::Delete => {
            // Backspace와 같은 이유로 no-op(문서 끝에서 Delete)에는 dirty를
            // 세우지 않는다. 다만 Delete는 캐럿이 제자리인 채로 실제 삭제가
            // 일어나므로(다음 문자/개행 제거), 캐럿 이동이 아니라 "삭제를
            // 수행했는가"를 직접 본다.
            let mut changed = sel.is_some();
            doc.text_caret = match delete_sel(&mut e.lines, sel) {
                Some(p) => p,
                None => {
                    // 캐럿 다음 한 문자. 줄 끝이면 다음 줄과 병합.
                    let next = apply_caret_move(&e.lines, caret, CaretMove::Right);
                    if next == caret {
                        caret // 문서 끝 — no-op
                    } else {
                        changed = true;
                        delete_range(&mut e.lines, caret, next)
                    }
                }
            };
            doc.text_sel = None;
            if changed {
                e.dirty = true;
            }
        }
        TextEditIntent::Move(mv, extend) => {
            let (new_caret, new_sel) = next_caret_and_sel(&e.lines, caret, sel, mv, extend);
            doc.text_caret = new_caret;
            doc.text_sel = new_sel;
        }
        TextEditIntent::SelectAll => {
            let (a, b) = whole_document_sel(&e.lines);
            doc.text_sel = Some((a, b));
            doc.text_caret = b;
        }
        TextEditIntent::Copy => {
            if let Some((a, b)) = sel {
                let (a, b) = normalize(a, b);
                let s = selection_text(&e.lines, a, b);
                *clipboard = s.clone();
                ui.output_mut(|o| o.copied_text = s);
            }
        }
        TextEditIntent::Cut => {
            if let Some((a, b)) = sel {
                let (a, b) = normalize(a, b);
                let s = selection_text(&e.lines, a, b);
                *clipboard = s.clone();
                ui.output_mut(|o| o.copied_text = s);
                doc.text_caret = delete_range(&mut e.lines, a, b);
                doc.text_sel = None;
                e.dirty = true;
            }
        }
        TextEditIntent::Paste(s) => {
            let s = s.replace("\r\n", "\n").replace('\r', "\n");
            if s.is_empty() {
                return;
            }
            let at = delete_sel(&mut e.lines, sel).unwrap_or(caret);
            doc.text_caret = insert_str(&mut e.lines, at, &s);
            doc.text_sel = None;
            e.dirty = true;
        }
    }
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
            parse_logical_line_edit(doc, 0, delim)
        } else {
            None
        };
        let data_start = if doc.has_header { 1 } else { 0 };
        const COL_COUNT_SAMPLE_ROWS: usize = 10;
        let mut col_count = header_fields.as_ref().map(|h| h.len()).unwrap_or(0);
        for logical in data_start..data_start + COL_COUNT_SAMPLE_ROWS {
            if let Some(fields) = parse_logical_line_edit(doc, logical, delim) {
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
        // parse_logical_line_edit이 ~로 분리해야 한다.
        let p = temp_ext(b"a~b~c\nd~e~f\n", "txt");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        doc.sep = SeparatorMode::Char(b'~');

        assert_eq!(
            parse_logical_line_edit(doc, 0, b'~'),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
        assert_eq!(compute_col_count(doc), 3);
    }

    #[test]
    fn enter_edit_mode_loads_lines() {
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        enter_edit_mode(doc);
        assert!(doc.edit.is_some());
        assert_eq!(doc.edit.as_ref().unwrap().lines, vec!["a,b", "1,2"]);
    }

    #[test]
    fn open_path_stores_real_path() {
        // 저장(덮어쓰기)이 표시 문자열을 되파싱하지 않고 쓸 수 있어야 한다.
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_ref().unwrap();
        assert_eq!(doc.path, p);
        assert_eq!(doc.path_label, p.display().to_string());
    }

    #[test]
    fn edit_dirty_reflects_buffer_state() {
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        // 파일만 열린 상태(뷰 모드)는 dirty가 아니다.
        assert!(!app.edit_dirty());
        {
            let doc = app.doc.as_mut().unwrap();
            doc.indexer.take().unwrap().join().unwrap();
            enter_edit_mode(doc);
        }
        assert!(!app.edit_dirty(), "막 진입한 버퍼는 깨끗하다");
        app.doc.as_mut().unwrap().edit.as_mut().unwrap().dirty = true;
        assert!(app.edit_dirty());
    }

    /// 편집 모드 정렬 테스트용 Document를 만든다(인덱싱 완료 + 편집 모드 진입).
    fn edit_doc(content: &[u8], has_header: bool) -> (App, u8) {
        let p = temp(content);
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        doc.has_header = has_header;
        enter_edit_mode(doc);
        (app, b',')
    }

    #[test]
    fn edit_sort_rearranges_lines_and_keeps_header() {
        let (mut app, delim) = edit_doc(b"name,n\nCharlie,3\nAlice,1\nBob,2\n", true);
        let doc = app.doc.as_mut().unwrap();
        let spec = SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true };
        apply_edit_sort(doc, &[spec], delim, 1);
        let e = doc.edit.as_ref().unwrap();
        assert_eq!(e.lines, v(&["name,n", "Alice,1", "Bob,2", "Charlie,3"]));
        assert!(e.dirty, "정렬은 버퍼를 변경한다");
    }

    #[test]
    fn edit_sort_does_not_set_view_sort_state() {
        // 편집 모드 정렬은 lines에 이미 반영되므로 살아 있는 permutation이 없다.
        // doc.sort를 세우면 헤더 화살표/상태바가 없는 정렬을 주장하고,
        // render_table이 permutation으로 행을 한 번 더 매핑해 순서가 깨진다.
        let (mut app, delim) = edit_doc(b"name,n\nCharlie,3\nAlice,1\n", true);
        let doc = app.doc.as_mut().unwrap();
        let spec = SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true };
        apply_edit_sort(doc, &[spec], delim, 1);
        assert!(doc.sort.is_none());
        assert!(doc.sort_job.is_none());
    }

    #[test]
    fn edit_sort_clears_row_pointing_state() {
        // 행이 뒤섞이면 선택/편집 중 셀이 가리키던 행이 달라진다 → 초기화.
        let (mut app, delim) = edit_doc(b"name,n\nCharlie,3\nAlice,1\n", true);
        let doc = app.doc.as_mut().unwrap();
        doc.cell_sel = Some((1, 0, 1, 1));
        doc.editing_cell = Some((1, 0));
        doc.cell_edit_text = "x".into();
        let spec = SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true };
        apply_edit_sort(doc, &[spec], delim, 1);
        assert!(doc.cell_sel.is_none());
        assert!(doc.editing_cell.is_none());
        assert!(doc.cell_edit_text.is_empty());
    }

    #[test]
    fn insert_after_edit_sort_stays_where_inserted() {
        // 정렬 뒤 행을 삽입하면 재정렬되지 않고 그 자리에 남아야 한다
        // (permutation이 아니라 물리적 재배치이므로 자연히 그렇다).
        let (mut app, delim) = edit_doc(b"name,n\nCharlie,3\nAlice,1\nBob,2\n", true);
        let doc = app.doc.as_mut().unwrap();
        let spec = SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true };
        apply_edit_sort(doc, &[spec], delim, 1);
        // "Bob,2"(index 2) 위에 zzz 행 삽입.
        let e = doc.edit.as_mut().unwrap();
        crate::edit::insert_row(&mut e.lines, 2, "zzz,9".into());
        assert_eq!(
            e.lines,
            v(&["name,n", "Alice,1", "zzz,9", "Bob,2", "Charlie,3"]),
            "삽입 행은 정렬 순서로 밀려나지 않는다"
        );
    }

    #[test]
    fn edit_sort_headerless_sorts_all_rows() {
        let (mut app, delim) = edit_doc(b"3,c\n1,a\n2,b\n", false);
        let doc = app.doc.as_mut().unwrap();
        let spec = SortSpec { col: 0, kind: SortKind::Number, dir: SortDir::Asc, ci: false };
        apply_edit_sort(doc, &[spec], delim, 0);
        assert_eq!(doc.edit.as_ref().unwrap().lines, v(&["1,a", "2,b", "3,c"]));
    }

    #[test]
    fn edit_sort_multi_key() {
        let (mut app, delim) = edit_doc(b"g,n\nb,2\na,2\nb,1\na,1\n", true);
        let doc = app.doc.as_mut().unwrap();
        let specs = vec![
            SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true },
            SortSpec { col: 1, kind: SortKind::Number, dir: SortDir::Desc, ci: false },
        ];
        apply_edit_sort(doc, &specs, delim, 1);
        assert_eq!(
            doc.edit.as_ref().unwrap().lines,
            v(&["g,n", "a,2", "a,1", "b,2", "b,1"])
        );
    }

    #[test]
    fn edit_sort_noop_on_empty_specs_or_data() {
        // 기준이 없거나 데이터 행이 없으면 아무것도 하지 않는다(dirty도 안 켠다).
        let (mut app, delim) = edit_doc(b"name,n\n", true);
        let doc = app.doc.as_mut().unwrap();
        apply_edit_sort(doc, &[], delim, 1);
        assert!(!doc.edit.as_ref().unwrap().dirty);
        let spec = SortSpec { col: 0, kind: SortKind::Text, dir: SortDir::Asc, ci: true };
        // 헤더만 있는 파일(데이터 행 0개).
        apply_edit_sort(doc, &[spec], delim, 1);
        assert!(!doc.edit.as_ref().unwrap().dirty);
    }

    #[test]
    fn sort_dialog_column_labels_use_edit_buffer() {
        // 브리프의 이월 결함: 다이얼로그가 mmap 전용 경로로 헤더를 읽으면
        // 편집 모드에서 편집 전 값이 나온다. parse_logical_line_edit 경유여야 한다.
        let (mut app, delim) = edit_doc(b"old,b\n1,2\n", true);
        let doc = app.doc.as_mut().unwrap();
        crate::edit::set_cell(&mut doc.edit.as_mut().unwrap().lines, 0, 0, "new", delim);
        assert_eq!(
            parse_logical_line_edit(doc, 0, delim),
            Some(v(&["new", "b"])),
            "편집한 헤더 값이 보여야 함"
        );
    }

    #[test]
    fn save_roundtrip_cp949_from_edit_buffer() {
        // 저장 다이얼로그가 부르는 조합(write_file + SaveOptions)이 편집 버퍼를
        // CP949로 변환해 쓰고, 다시 열면 같은 내용이 나오는지.
        let (mut app, _delim) = edit_doc("h,v\n가,1\n".as_bytes(), true);
        let out = temp_ext(b"", "csv");
        {
            let e = app.doc.as_ref().unwrap().edit.as_ref().unwrap();
            let opts = crate::save::SaveOptions {
                enc: crate::parse::Encoding::Cp949,
                bom: false,
                newline: e.newline,
            };
            crate::save::write_file(&out, &e.lines, &opts, None).unwrap();
        }
        let bytes = std::fs::read(&out).unwrap();
        // CP949 '가' = B0 A1
        assert!(bytes.windows(2).any(|w| w == [0xB0, 0xA1]), "CP949로 인코딩됨");
        // 다시 열어(CP949로) 같은 줄이 나오는지.
        let ctx = egui::Context::default();
        let mut app2 = App::default();
        app2.open_path(&out, &ctx);
        let doc2 = app2.doc.as_mut().unwrap();
        doc2.indexer.take().unwrap().join().unwrap();
        doc2.enc = crate::parse::Encoding::Cp949;
        enter_edit_mode(doc2);
        assert_eq!(doc2.edit.as_ref().unwrap().lines, v(&["h,v", "가,1"]));
    }

    /// 저장 후 뷰 경로(mmap)가 낡지 않는지. 리뷰가 지목한 CRITICAL 결함의 회귀 방지:
    /// `Source`가 붙들고 있는 `Mmap`은 `write_file`의 `rename` 뒤에도 **저장 전
    /// 바이트**를 계속 돌려주므로, 저장 직후 편집 모드를 끄면 화면이 편집 전
    /// 내용으로 되돌아간다. `repoint_source_after_save`가 이를 막아야 한다.
    #[test]
    fn save_repoints_source_so_view_mode_shows_saved_content() {
        let (mut app, delim) = edit_doc(b"h,v\nold,1\n", true);
        let ctx = egui::Context::default();
        let path = app.doc.as_ref().unwrap().path.clone();

        // 편집: 셀 하나를 바꾼다.
        {
            let doc = app.doc.as_mut().unwrap();
            crate::edit::set_cell(&mut doc.edit.as_mut().unwrap().lines, 1, 0, "new", delim);
            doc.edit.as_mut().unwrap().dirty = true;
        }

        // 저장 다이얼로그가 하는 일과 같은 순서: write_file → dirty 해제 → 소스 재지정.
        {
            let doc = app.doc.as_mut().unwrap();
            let e = doc.edit.as_ref().unwrap();
            let opts = crate::save::SaveOptions {
                enc: doc.enc,
                bom: false,
                newline: e.newline,
            };
            crate::save::write_file(&path, &e.lines, &opts, None).unwrap();
            doc.edit.as_mut().unwrap().dirty = false;
            repoint_source_after_save(doc, &path, &ctx).unwrap();
            doc.indexer.take().unwrap().join().unwrap();
        }

        let doc = app.doc.as_mut().unwrap();
        // 매핑된 바이트 자체가 저장된 내용이어야 한다(옛 매핑이면 "old,1"이 보인다).
        let raw = String::from_utf8(doc.source.as_bytes().to_vec()).unwrap();
        assert!(raw.contains("new,1"), "mmap이 저장된 내용을 보여야 함: {raw:?}");
        assert!(!raw.contains("old,1"), "저장 전 내용이 남아 있으면 안 됨: {raw:?}");

        // 편집 모드를 끄면 뷰 경로(decode_logical_line)로 떨어진다 — 저장된 내용이어야.
        exit_edit_mode(doc);
        assert!(doc.edit.is_none());
        assert_eq!(decode_logical_line(doc, 1).as_deref(), Some("new,1"));
        assert_eq!(logical_line(doc, 1).as_deref(), Some("new,1"));
        // 인덱스 행 수도 새 파일과 맞아야 한다(낡은 인덱스면 유령 행이 남는다).
        assert_eq!(doc.index.line_count(), 2);
    }

    /// 소스 재지정이 편집 세션을 건드리지 않는지. 사용자는 저장 후에도 편집 모드에
    /// 남아 계속 편집할 수 있어야 하고, `edit.lines`는 절대 디스크에서 다시
    /// 읽히면 안 된다(만약 내용이 달랐다면 편집 내용을 잃게 된다).
    #[test]
    fn repoint_after_save_preserves_edit_buffer_and_selection() {
        let (mut app, _delim) = edit_doc(b"h,v\na,1\nb,2\n", true);
        let ctx = egui::Context::default();
        let path = app.doc.as_ref().unwrap().path.clone();
        let doc = app.doc.as_mut().unwrap();
        doc.cell_sel = Some((1, 0, 2, 1));
        doc.selected_col = Some(1);
        // 디스크에는 버퍼와 다른 내용을 써 둔다 — 재지정이 버퍼를 덮어쓰면 들킨다.
        // 열린 mmap 때문에 in-place 쓰기(fs::write)는 Windows에서 실패하므로
        // (ERROR_USER_MAPPED_FILE), 프로덕션과 같은 rename 경로(write_file)를 쓴다.
        {
            let opts = crate::save::SaveOptions {
                enc: doc.enc,
                bom: false,
                newline: crate::edit::Newline::Lf,
            };
            crate::save::write_file(&path, &v(&["h,v", "DISK,9"]), &opts, None).unwrap();
        }

        repoint_source_after_save(doc, &path, &ctx).unwrap();
        doc.indexer.take().unwrap().join().unwrap();

        let e = doc.edit.as_ref().expect("편집 모드가 유지되어야 함");
        assert_eq!(e.lines, v(&["h,v", "a,1", "b,2"]), "버퍼는 디스크에서 다시 읽지 않는다");
        assert_eq!(doc.cell_sel, Some((1, 0, 2, 1)), "선택 유지");
        assert_eq!(doc.selected_col, Some(1), "선택 컬럼 유지");
        // 편집 모드에서는 여전히 버퍼가 진실.
        assert_eq!(logical_line(doc, 1).as_deref(), Some("a,1"));
    }

    /// save-as: 새 경로로 저장하면 소스도 새 파일을 매핑해야 한다
    /// (예전에는 path만 새 파일을 가리키고 source는 원본을 매핑한 채였다).
    #[test]
    fn save_as_repoints_source_to_new_path() {
        let (mut app, delim) = edit_doc(b"h,v\nold,1\n", true);
        let ctx = egui::Context::default();
        let out = temp_ext(b"", "csv");
        {
            let doc = app.doc.as_mut().unwrap();
            crate::edit::set_cell(&mut doc.edit.as_mut().unwrap().lines, 1, 0, "new", delim);
            let e = doc.edit.as_ref().unwrap();
            let opts = crate::save::SaveOptions {
                enc: doc.enc,
                bom: false,
                newline: e.newline,
            };
            crate::save::write_file(&out, &e.lines, &opts, None).unwrap();
            doc.path_label = out.display().to_string();
            doc.path = out.clone();
            repoint_source_after_save(doc, &out, &ctx).unwrap();
            doc.indexer.take().unwrap().join().unwrap();
        }
        let doc = app.doc.as_mut().unwrap();
        exit_edit_mode(doc);
        assert_eq!(decode_logical_line(doc, 1).as_deref(), Some("new,1"));
        assert_eq!(doc.path, out);
    }

    /// 저장 실패해도 편집 버퍼는 dirty로 남는지(Finding 3의 내부 상태 쪽).
    /// 표시 자체(`ui.colored_label` 배치)는 GUI 없이 검증할 수 없어 수동
    /// 체크리스트 F-5로 넘겼다 — 여기서는 상태바가 읽는 값이 살아 있음을 고정한다.
    #[test]
    fn failed_save_keeps_dirty_state_visible_to_status_bar() {
        let (mut app, delim) = edit_doc(b"h,v\nold,1\n", true);
        {
            let doc = app.doc.as_mut().unwrap();
            crate::edit::set_cell(&mut doc.edit.as_mut().unwrap().lines, 1, 0, "new", delim);
            doc.edit.as_mut().unwrap().dirty = true;
        }
        // 존재하지 않는 디렉터리로 저장 → 실패.
        let bad = std::path::Path::new("no_such_dir_xyz").join("out.csv");
        let doc = app.doc.as_ref().unwrap();
        let e = doc.edit.as_ref().unwrap();
        let opts = crate::save::SaveOptions { enc: doc.enc, bom: false, newline: e.newline };
        assert!(crate::save::write_file(&bad, &e.lines, &opts, None).is_err());
        // 상태바가 읽는 두 값이 모두 살아 있어야 한다.
        assert!(app.edit_dirty(), "실패한 저장은 dirty를 유지한다");
        assert_eq!(app.doc.as_ref().unwrap().edit.as_ref().unwrap().lines.len(), 2);
    }

    #[test]
    fn normalize_rect_orders_corners() {
        // 아래→위, 오른쪽→왼쪽으로 드래그해도 (r0<=r1, c0<=c1)로 정규화.
        assert_eq!(normalize_rect((5, 4, 2, 1)), (2, 1, 5, 4));
        assert_eq!(normalize_rect((2, 1, 5, 4)), (2, 1, 5, 4));
        // 역방향 혼합(행은 정방향, 열은 역방향).
        assert_eq!(normalize_rect((2, 4, 5, 1)), (2, 1, 5, 4));
    }

    #[test]
    fn rect_contains_respects_unnormalized_input() {
        // 역방향으로 저장된 선택도 포함 판정이 정확해야 한다(우클릭 셀이
        // 선택 안인지 밖인지 판단하는 데 쓰임).
        let sel = (5, 4, 2, 1); // 실제 범위: 행 2..=5, 열 1..=4
        assert!(rect_contains(sel, 3, 2));
        assert!(rect_contains(sel, 2, 1));
        assert!(rect_contains(sel, 5, 4));
        assert!(!rect_contains(sel, 1, 2), "행 범위 밖");
        assert!(!rect_contains(sel, 3, 0), "열 범위 밖");
        assert!(!rect_contains(sel, 6, 4), "행 범위 밖");
    }

    #[test]
    fn cell_drag_active_latches_only_on_cell_press() {
        // 버튼을 뗀 프레임은 무조건 해제.
        assert!(!next_cell_drag_active(true, false, false));
        assert!(!next_cell_drag_active(true, false, true));
        // 표 밖에서 누른 채 표 위로 들어온 포인터: 셀 누름이 관측된 적 없으므로
        // 버튼이 눌려 있어도 켜지지 않는다(선택이 끌려가지 않음).
        assert!(!next_cell_drag_active(false, true, false));
        // 셀 위에서 누름이 시작되면 켜진다.
        assert!(next_cell_drag_active(false, true, true));
        // 켜진 뒤에는 포인터가 셀 밖으로 나가도 버튼이 눌린 동안 유지된다.
        assert!(next_cell_drag_active(true, true, false));
        // 뗐다가 다시 누르면 셀 누름이 다시 관측돼야만 켜진다.
        let after_release = next_cell_drag_active(true, false, false);
        assert!(!next_cell_drag_active(after_release, true, false));
    }

    fn v(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    fn tp(line: usize, col: usize) -> crate::edit::TextPos {
        crate::edit::TextPos { line, col }
    }

    #[test]
    fn clamp_pos_bounds_line_and_col() {
        let lines = v(&["ab", "cdef"]);
        assert_eq!(clamp_pos(&lines, tp(9, 9)), tp(1, 4));
        assert_eq!(clamp_pos(&lines, tp(0, 9)), tp(0, 2));
        assert_eq!(clamp_pos(&lines, tp(1, 2)), tp(1, 2));
        // 빈 lines도 안전하게.
        assert_eq!(clamp_pos(&[], tp(3, 3)), tp(0, 0));
    }

    #[test]
    fn caret_move_crosses_line_boundaries() {
        let lines = v(&["ab", "cd"]);
        // 줄 끝에서 오른쪽 → 다음 줄 처음.
        assert_eq!(apply_caret_move(&lines, tp(0, 2), CaretMove::Right), tp(1, 0));
        // 줄 처음에서 왼쪽 → 앞 줄 끝.
        assert_eq!(apply_caret_move(&lines, tp(1, 0), CaretMove::Left), tp(0, 2));
        // 문서 처음/끝에서는 no-op(끝은 마지막 줄 끝으로 클램프).
        assert_eq!(apply_caret_move(&lines, tp(0, 0), CaretMove::Left), tp(0, 0));
        assert_eq!(apply_caret_move(&lines, tp(1, 2), CaretMove::Right), tp(1, 2));
    }

    #[test]
    fn caret_move_up_down_clamps_column() {
        let lines = v(&["abcdef", "xy"]);
        // 긴 줄에서 짧은 줄로 내려가면 col이 줄 길이로 클램프.
        assert_eq!(apply_caret_move(&lines, tp(0, 5), CaretMove::Down), tp(1, 2));
        // 첫 줄에서 위 → 문서 처음.
        assert_eq!(apply_caret_move(&lines, tp(0, 3), CaretMove::Up), tp(0, 0));
        // 마지막 줄에서 아래 → 그 줄 끝.
        assert_eq!(apply_caret_move(&lines, tp(1, 1), CaretMove::Down), tp(1, 2));
    }

    #[test]
    fn caret_move_home_end() {
        let lines = v(&["hello"]);
        assert_eq!(apply_caret_move(&lines, tp(0, 3), CaretMove::Home), tp(0, 0));
        assert_eq!(apply_caret_move(&lines, tp(0, 1), CaretMove::End), tp(0, 5));
    }

    #[test]
    fn shift_arrow_extends_keeping_anchor() {
        let lines = v(&["abcdef"]);
        // 선택 없음 + Shift+Right → 앵커는 현재 캐럿.
        let (c, s) = next_caret_and_sel(&lines, tp(0, 2), None, CaretMove::Right, true);
        assert_eq!(c, tp(0, 3));
        assert_eq!(s, Some((tp(0, 2), tp(0, 3))));
        // 이어서 한 번 더 → 앵커 유지, 캐럿만 전진.
        let (c2, s2) = next_caret_and_sel(&lines, c, s, CaretMove::Right, true);
        assert_eq!(c2, tp(0, 4));
        assert_eq!(s2, Some((tp(0, 2), tp(0, 4))));
        // 되돌아와 앵커와 같아지면 선택 해제.
        let (_, s3) = next_caret_and_sel(&lines, tp(0, 3), s, CaretMove::Left, true);
        assert_eq!(s3, None);
    }

    #[test]
    fn plain_arrow_collapses_selection_without_moving() {
        let lines = v(&["abcdef"]);
        let sel = Some((tp(0, 1), tp(0, 4)));
        // 왼쪽 → 선택 시작으로 붕괴(한 칸 더 가지 않는다).
        assert_eq!(
            next_caret_and_sel(&lines, tp(0, 4), sel, CaretMove::Left, false),
            (tp(0, 1), None)
        );
        // 오른쪽 → 선택 끝으로 붕괴.
        assert_eq!(
            next_caret_and_sel(&lines, tp(0, 4), sel, CaretMove::Right, false),
            (tp(0, 4), None)
        );
        // 역방향 선택도 정규화해서 판단.
        let rev = Some((tp(0, 4), tp(0, 1)));
        assert_eq!(
            next_caret_and_sel(&lines, tp(0, 1), rev, CaretMove::Right, false),
            (tp(0, 4), None)
        );
    }

    #[test]
    fn plain_arrow_moves_when_no_selection() {
        let lines = v(&["abc"]);
        assert_eq!(
            next_caret_and_sel(&lines, tp(0, 1), None, CaretMove::Right, false),
            (tp(0, 2), None)
        );
    }

    #[test]
    fn select_all_spans_whole_document() {
        let lines = v(&["ab", "cde"]);
        assert_eq!(whole_document_sel(&lines), (tp(0, 0), tp(1, 3)));
        // 빈 문서(줄 없음)에서도 패닉하지 않는다.
        assert_eq!(whole_document_sel(&[]), (tp(0, 0), tp(0, 0)));
    }

    #[test]
    fn sel_span_covers_first_middle_last_lines() {
        // 0행 col2 ~ 2행 col1 선택. 중간 줄은 전부, 시작/끝 줄은 부분.
        let a = tp(0, 2);
        let b = tp(2, 1);
        // 시작 줄: col2부터 줄 끝(len=5) + 개행 한 칸 → (2, 6)
        assert_eq!(sel_span_on_line(a, b, 0, 5), Some((2, 6)));
        // 중간 줄: 전부 + 개행.
        assert_eq!(sel_span_on_line(a, b, 1, 3), Some((0, 4)));
        // 끝 줄: 처음부터 col1까지(개행 없음).
        assert_eq!(sel_span_on_line(a, b, 2, 7), Some((0, 1)));
        // 범위 밖 줄.
        assert_eq!(sel_span_on_line(a, b, 3, 4), None);
    }

    #[test]
    fn sel_span_single_line_and_empty() {
        let a = tp(1, 1);
        let b = tp(1, 3);
        assert_eq!(sel_span_on_line(a, b, 1, 5), Some((1, 3)));
        // 같은 줄 빈 범위 → None(캐럿만 있는 상태).
        assert_eq!(sel_span_on_line(a, a, 1, 5), None);
        // 끝 줄의 col이 0이면(다음 줄 처음에서 끝나는 선택) 그 줄엔 음영 없음.
        assert_eq!(sel_span_on_line(tp(0, 0), tp(1, 0), 1, 5), None);
    }

    /// Backspace의 dirty 판정. `apply_text_intent`가 쓰는 것과 똑같이
    /// `edit::backspace`의 반환 위치를 헬퍼에 먹여, 실제 no-op 조건에서
    /// dirty가 서지 않음을 고정한다(파일 열자마자 Backspace → "저장 안 됨" 방지).
    #[test]
    fn backspace_at_origin_is_not_dirty() {
        // 문서 맨 앞 + 선택 없음 → edit::backspace가 위치를 그대로 돌려주고,
        // 버퍼도 그대로다. 변경 없음으로 판정해야 한다.
        let mut lines = v(&["ab", "cd"]);
        let before = tp(0, 0);
        let after = crate::edit::backspace(&mut lines, before);
        assert_eq!(lines, v(&["ab", "cd"]), "버퍼가 바뀌면 안 된다");
        assert!(!backspace_or_delete_changed(false, before, after));
    }

    #[test]
    fn backspace_that_deletes_is_dirty() {
        // (a) 줄 중간: 한 글자 삭제 → 캐럿이 뒤로 → 변경.
        let mut lines = v(&["ab"]);
        let before = tp(0, 2);
        let after = crate::edit::backspace(&mut lines, before);
        assert_eq!(lines, v(&["a"]));
        assert!(backspace_or_delete_changed(false, before, after));

        // (b) 줄 맨 앞: 앞 줄과 병합 → 캐럿이 앞 줄로 → 변경.
        let mut lines = v(&["ab", "cd"]);
        let before = tp(1, 0);
        let after = crate::edit::backspace(&mut lines, before);
        assert_eq!(lines, v(&["abcd"]));
        assert!(backspace_or_delete_changed(false, before, after));

        // (c) 선택이 있으면 캐럿이 제자리여도(선택 시작 == 캐럿) 변경이다.
        assert!(backspace_or_delete_changed(true, tp(0, 0), tp(0, 0)));
    }

    /// Delete의 no-op 조건: 문서 끝에서는 Right 이동이 제자리라 삭제가 없다.
    /// `apply_text_intent`의 Delete 분기가 이 조건으로 dirty를 가른다.
    #[test]
    fn delete_at_document_end_is_noop() {
        let lines = v(&["ab", "cd"]);
        let caret = tp(1, 2); // 마지막 줄 끝
        assert_eq!(apply_caret_move(&lines, caret, CaretMove::Right), caret);
        // 문서 끝이 아니면 이동이 생기고 → 삭제할 대상이 있다.
        let mid = tp(0, 2); // 첫 줄 끝(개행 위치)
        assert_ne!(apply_caret_move(&lines, mid, CaretMove::Right), mid);
    }

    /// Tab이 본문 텍스트 줄로 포커스를 옮길 수 없어야 한다.
    ///
    /// `render_text`의 키 입력 게이트는 `memory.focused().is_none()`이다.
    /// 줄이 포커스를 가져갈 수 있으면 Tab 한 번으로 게이트가 닫혀 **모든 키
    /// 입력이 조용히 삼켜지고 문서가 편집 불가**가 된다(화면에 아무 표시도 없다).
    ///
    /// 방어가 두 겹인 이유를 이 테스트가 고정한다:
    /// 1. `TEXT_LINE_SENSE`의 `focusable: false` — Tab 순회 등록 자체를 막는다.
    /// 2. `surrender_focus` — `context_menu`가 내부에서
    ///    `interact(Sense::click())`(focusable: true)로 sense를 union해
    ///    줄을 **다시** focusable로 등록하는 것을 되돌린다.
    ///
    /// 2번 없이 1번만으로는 실제로 막히지 않는다(이 테스트로 실증했다).
    #[test]
    fn tab_cannot_steal_focus_onto_text_line() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("textline_focus_probe");
        // render_text가 한 줄에 대해 하는 것과 같은 순서: interact → context_menu
        // → surrender_focus.
        let draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect =
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 20.0));
                let resp = ui.interact(rect, id, TEXT_LINE_SENSE);
                resp.context_menu(|ui| {
                    ui.label("메뉴");
                });
                ui.memory_mut(|m| m.surrender_focus(id));
            });
        };
        // 1프레임: 위젯 등록.
        let _ = ctx.run(Default::default(), |ctx| draw(ctx));
        // 2프레임: Tab.
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(input, |ctx| draw(ctx));

        assert_eq!(
            ctx.memory(|m| m.focused()),
            None,
            "Tab이 텍스트 줄에 포커스를 걸면 keyboard_free 게이트가 닫혀 \
             문서 전체가 편집 불가가 된다"
        );
    }

    /// 게이트 자체의 의미 고정: 포커스가 있으면 키를 소비하지 않는다.
    /// (툴바 "직접:" 구분자 TextEdit에 타이핑할 때 본문에 중복 입력되지 않게 하는
    ///  원래 목적 — I-1 수정이 이 성질을 깨지 않았음을 확인한다.)
    #[test]
    fn focused_widget_blocks_document_key_intents() {
        let ctx = egui::Context::default();
        let other = egui::Id::new("toolbar_textedit");
        // 포커스를 가진 다른 위젯이 있는 상태를 만든다.
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect =
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(50.0, 20.0));
                let r = ui.interact(rect, other, egui::Sense::click());
                r.request_focus();
            });
        });
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(other),
            "사전 조건: 다른 위젯이 포커스를 쥐고 있어야 한다"
        );
        // render_text의 게이트 식과 동일.
        let keyboard_free = ctx.memory(|m| m.focused().is_none());
        assert!(!keyboard_free, "포커스가 있으면 본문은 키를 소비하지 않는다");
    }

    /// Tab이 표 모드 데이터 셀로 포커스를 옮길 수 없어야 한다.
    /// `tab_cannot_steal_focus_onto_text_line`의 표 모드 판.
    ///
    /// 표 모드에는 `render_text`의 `keyboard_free` 같은 전역 게이트가 없어
    /// 오늘 당장 편집 불가가 되지는 않지만, 구조적 결함은 동일하다
    /// (`Sense::click_and_drag()` = focusable: true → 그려진 셀마다 Tab 순회
    /// 등록, 그리고 `context_menu`가 `Sense::click()`을 union해 그 opt-out을
    /// 매 프레임 무효화). `TABLE_CELL_SENSE` + `surrender_focus` 두 겹이
    /// 유지되는지를 고정한다.
    #[test]
    fn tab_cannot_steal_focus_onto_table_cell() {
        let ctx = egui::Context::default();
        let id = egui::Id::new(("cell", 3usize, 2usize));
        // render_table이 한 셀에 대해 하는 것과 같은 순서:
        // interact → context_menu → surrender_focus.
        let draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect =
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(80.0, 18.0));
                let resp = ui.interact(rect, id, TABLE_CELL_SENSE);
                resp.context_menu(|ui| {
                    ui.label("메뉴");
                });
                ui.memory_mut(|m| m.surrender_focus(id));
            });
        };
        // 1프레임: 위젯 등록.
        let _ = ctx.run(Default::default(), |ctx| draw(ctx));
        // 2프레임: Tab.
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(input, |ctx| draw(ctx));

        assert_eq!(
            ctx.memory(|m| m.focused()),
            None,
            "Tab이 데이터 셀에 포커스를 걸면 안 된다"
        );
    }

    /// 셀 fix가 인라인 셀 편집기의 포커스를 깨지 않는지 고정한다.
    ///
    /// `render_table`은 편집 중인 셀에서 **early return** 하므로 그 셀의
    /// `interact`/`context_menu`/`surrender_focus`는 아예 실행되지 않는다.
    /// 게다가 편집기 id(`("celledit", ..)`)는 셀 상호작용 id(`("cell", ..)`)와
    /// 다르고, `Memory::surrender_focus`는 그 id가 포커스일 때만 지운다
    /// (`memory.rs:762-767`). 즉 **다른** 셀들이 매 프레임 부르는
    /// `surrender_focus`가 편집 중인 TextEdit의 포커스를 빼앗을 수 없다.
    #[test]
    fn cell_surrender_focus_does_not_disturb_inline_editor() {
        let ctx = egui::Context::default();
        let editor = egui::Id::new(("celledit", 3usize, 2usize));
        // 이웃 셀들 — 편집 중이 아니므로 interact + surrender_focus를 돈다.
        let neighbors = [
            egui::Id::new(("cell", 3usize, 1usize)),
            egui::Id::new(("cell", 4usize, 2usize)),
        ];
        let draw = |ctx: &egui::Context, request: bool| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // 편집 중인 셀: TextEdit이 포커스를 요청한다(render_table과 동일).
                let mut buf = String::from("값");
                let resp = ui.add(egui::TextEdit::singleline(&mut buf).id(editor));
                if request {
                    resp.request_focus();
                }
                // 나머지 셀들: 셀 상호작용 + 메뉴 + 포커스 반납.
                for (i, nid) in neighbors.iter().enumerate() {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(0.0, 40.0 + 20.0 * i as f32),
                        egui::vec2(80.0, 18.0),
                    );
                    let r = ui.interact(rect, *nid, TABLE_CELL_SENSE);
                    r.context_menu(|ui| {
                        ui.label("메뉴");
                    });
                    ui.memory_mut(|m| m.surrender_focus(*nid));
                }
            });
        };
        // 1프레임: 편집기가 포커스를 잡는다.
        let _ = ctx.run(Default::default(), |ctx| draw(ctx, true));
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(editor),
            "사전 조건: 인라인 셀 편집기가 포커스를 쥐어야 한다"
        );
        // 2프레임: 편집기는 더 이상 요청하지 않는다(render_table도 has_focus면
        // 요청하지 않는다). 이웃 셀들의 surrender_focus가 돌아도 포커스는 유지돼야 한다.
        let _ = ctx.run(Default::default(), |ctx| draw(ctx, false));
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(editor),
            "다른 셀의 surrender_focus가 편집 중인 TextEdit의 포커스를 빼앗으면 안 된다"
        );
    }

    #[test]
    fn logical_line_reads_from_edit_buffer() {
        let p = temp(b"a,b\n1,2\n");
        let ctx = egui::Context::default();
        let mut app = App::default();
        app.open_path(&p, &ctx);
        let doc = app.doc.as_mut().unwrap();
        doc.indexer.take().unwrap().join().unwrap();
        enter_edit_mode(doc);
        // 편집 버퍼 값을 바꾸면 logical_line도 그 값을 반환.
        doc.edit.as_mut().unwrap().lines[1] = "X,Y".to_string();
        assert_eq!(logical_line(doc, 1).as_deref(), Some("X,Y"));
    }
}
