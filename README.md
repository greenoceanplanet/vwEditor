# textViewer

대용량 CSV/TSV/텍스트 파일을 즉시 여는 뷰어 겸 에디터. Rust + egui.

10GB 파일을 열어도 첫 화면이 곧바로 뜬다. 파일 전체를 읽지 않고 앞부분만
훑어 표시한 뒤, 나머지 줄 위치는 백그라운드에서 인덱싱한다. 스크롤은 읽은
데까지 따라 늘어난다.

> **상태:** 개인 도구로 만들어 쓰는 중이다. Windows에서 실사용하며 다듬었고,
> 825개 테스트가 붙어 있다. macOS·Linux는 빌드는 되지만 **실기 검증을 못 했다**
> — 아래 [플랫폼 지원](#플랫폼-지원) 참고.

## 기능

- **대용량 파일** — mmap + 백그라운드 줄 인덱싱. 10.8GB / 3억 행 정렬 5.9초(실측).
- **표 모드 / 텍스트 모드** — 구분자를 감지해 표로 보여주거나, 원문 그대로 본다.
- **자동 감지** — 인코딩(BOM → UTF-8 → CP949), 구분자, 헤더 유무. 전부 수동으로 덮어쓸 수 있다.
- **찾기·바꾸기** — 대소문자·전체 셀 일치 옵션. 전체 바꾸기는 899MB / 15.4M 행에서 약 0.25초.
- **정렬** — 다중 키 정렬. 문자열/숫자 판별.
- **컬럼 선택** — 볼 컬럼만 골라 표시.
- **편집·저장** — 인코딩·개행(CRLF/LF)·BOM을 골라 저장. Undo/Redo.
- **헥스 모드** — 바이너리 파일을 16진수로 본다(읽기 전용).
- **Parquet / GeoParquet** — 읽기 전용. geometry 컬럼은 `POINT(127.02 37.51)`,
  `POLYGON(1,204 pts)` 형태로 요약해 보여준다. CSV/TSV로 내보내기 가능.
- **파싱 오류 행 검출** — 따옴표가 안 닫힌 행 등을 찾아 준다.
- **멀티탭**, **드래그앤드롭**, **Ctrl+휠 확대**(0.5~4.0배).

## 빌드

Rust 툴체인이 필요하다([rustup](https://rustup.rs/)).

```powershell
cargo build --release
```

결과물은 `target/release/textviewer.exe` 하나다. 별도 런타임이 필요 없다.

### Linux 빌드 의존성

GUI 라이브러리 개발 패키지가 필요하다.

```bash
# Debian / Ubuntu
sudo apt install build-essential libgtk-3-dev libxkbcommon-dev

# Fedora
sudo dnf install gtk3-devel libxkbcommon-devel
```

## 플랫폼 지원

**Windows에서 만들었고 Windows에서만 실사용 검증했다.** 나머지 둘은 코드상
크로스 플랫폼이지만 실제로 돌려 보지 못했다.

| | 빌드 | 실기 검증 | 비고 |
|---|:---:|:---:|---|
| Windows | ✅ | ✅ | 개발·상용 환경 |
| macOS | ✅ | ❌ | 단축키가 Ctrl (맥 관례는 Cmd) |
| Linux | ✅ | ❌ | 개발 패키지 필요, 한글 폰트 확인 필요 |

의존성은 전부 크로스 플랫폼이다(eframe/egui, memmap2, rfd, rayon, parquet,
mimalloc). Windows 전용 크레이트는 쓰지 않는다.

### 폰트에 대해

**폰트를 앱에 내장하지 않는다.** 실행 파일과 저장소를 가볍게 유지하려는
선택이다. 대신 시스템 폰트를 찾아 쓴다 —
[`src/theme.rs`](src/theme.rs)의 `MONO_CANDIDATES` / `UI_CANDIDATES` /
`KR_CANDIDATES`에 플랫폼별 경로 목록이 있다.

UI가 한국어라 **한글 폰트가 없으면 글자가 □로 보인다.** egui 내장 폰트에는
CJK 글리프가 없기 때문이다. 이 경우 앱이 시작할 때 설치 방법을 안내한다.

```bash
# Debian / Ubuntu
sudo apt install fonts-nanum

# Fedora
sudo dnf install nhn-nanum-fonts

# Arch
sudo pacman -S noto-fonts-cjk
```

설치했는데도 두부가 보이면 그 폰트 경로가 목록에 없는 것이다.
`KR_CANDIDATES`에 경로를 추가하면 된다. **PR 환영** — 배포판마다 경로가
달라서 목록이 아직 성기다.

### macOS 사용자에게

단축키가 `Ctrl` 기준이라 맥 관례(`Cmd`)와 어긋난다. 맥에서도 Ctrl 키는
동작하므로 못 쓸 정도는 아니지만 어색하다. egui의 `Modifiers::command`는
플랫폼별로 Ctrl/Cmd를 자동 매핑하므로, 현재 `ctrl` 검사를 그것으로 바꾸면
양쪽이 동시에 맞는다. 이것 역시 **PR 환영**이다.

새 파일 기본 개행이 CRLF인 점도 맥·리눅스에서는 어색할 수 있다. 저장
다이얼로그에서 LF로 바꿀 수 있다.

## 기여

맥·리눅스 쪽은 위에 적은 대로 손댈 곳이 남아 있고, 저는 그 환경에서 검증할
수단이 없다. 그쪽 수정은 특히 반갑다.

- 코드 주석과 문서는 한국어다.
- 테스트를 붙여 주면 좋다 (`cargo test`).
- `.readme/` 에 작업 기록이 날짜순으로 쌓여 있다. 어떤 판단으로 지금 구조가
  나왔는지 대부분 거기 적혀 있다.

## 개발

```powershell
cargo test              # 825개
cargo clippy --all-targets
cargo build --release
```

## 라이선스

[GNU General Public License v3.0](LICENSE).

이 프로그램을 수정해 배포하면 그 소스도 같은 GPL v3으로 공개해야 한다.
개인적으로 고쳐 쓰는 것은 자유다.
