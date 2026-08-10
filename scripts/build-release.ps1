# 배포용 릴리스 빌드. **배포할 exe는 반드시 이 스크립트로 만든다.**
#
# 그냥 `cargo build --release` 로 만들면 실행 파일에 빌드한 사람의 홈
# 디렉터리가 박힌다. Rust 가 패닉 메시지·역추적용으로 소스 경로를 심기
# 때문인데, 소스가 깨끗해도 다음 같은 문자열이 남는다:
#
#   C:\Users\<사용자명>\.cargo\registry\src\...
#   C:\Users\<사용자명>\.rustup\toolchains\...
#   <프로젝트 절대 경로>\target\release\build\...
#
# 실제로 v1.0.0 바이너리에서 Windows 사용자명과 폴더 구조가 노출됐다.
#
# `--remap-path-prefix` 가 컴파일 시 그 경로를 치환한다. 파일명과 줄번호는
# 그대로 남으므로 패닉이 어디서 났는지는 여전히 알 수 있다.
#
# 이 일을 .cargo\config.toml 에 두지 않는 이유: 설정 파일은 환경 변수를
# 펼치지 못해 홈 경로를 적어 넣어야 하고, 그러면 지우려던 경로가 추적되는
# 파일로 옮겨갈 뿐이다.

$ErrorActionPreference = 'Stop'

$cargoHome  = if ($env:CARGO_HOME)  { $env:CARGO_HOME }  else { Join-Path $env:USERPROFILE '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
$projectDir = Split-Path -Parent $PSScriptRoot

$flags = @(
    "--remap-path-prefix=$cargoHome=/cargo"
    "--remap-path-prefix=$rustupHome=/rustc"
    "--remap-path-prefix=$projectDir=/project"
    "--remap-path-prefix=$env:USERPROFILE=/home"
) -join ' '

Write-Host "remapping build paths, then building release..." -ForegroundColor Cyan
$env:RUSTFLAGS = $flags
# cargo 는 경고를 stderr 로 낸다. ErrorActionPreference='Stop' 아래에서는 그것만으로
# 스크립트가 죽으므로, 이 구간에서만 끄고 종료 코드로 성공을 판단한다.
$ErrorActionPreference = 'Continue'
cargo build --release
$code = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
Remove-Item Env:\RUSTFLAGS
if ($code -ne 0) { throw "cargo build 실패 (exit $code)" }

$exe = Join-Path $projectDir 'target\release\vweditor.exe'
$hash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()

# 경로가 정말 지워졌는지 확인한다. 여기서 걸리면 배포하면 안 된다.
# PowerShell 5.1 에는 Encoding::Latin1 이 없다. 코드페이지 28591 이 같은 것이고,
# 여기서는 바이트를 1:1 로 문자에 대응시키기만 하면 되므로 이걸로 충분하다.
#
# 무엇을 찾는가: **누가·어디서 빌드했는지 알려주는 문자열**만 본다.
# remap 후 `/home\.cargo\registry\...` 같은 경로는 그대로 남지만 그것은
# 어느 머신에서나 같은 값이라 문제가 아니다. `.rustup` 이나 `.cargo` 자체를
# 금지어로 두면 정상 빌드가 매번 걸린다.
$bytes = [System.IO.File]::ReadAllBytes($exe)
$text  = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
$leaks = @()
$needles = @(
    $env:USERNAME              # 사용자명
    "$env:USERPROFILE"         # C:\Users\<이름>
    'C:\Users\'                # 남의 홈이라도 절대 경로면 새는 것
    $projectDir                # 프로젝트 절대 경로
)
foreach ($needle in $needles) {
    if ($needle -and $text.Contains($needle)) { $leaks += $needle }
}

Write-Host ""
Write-Host "exe    : $exe"
Write-Host "sha256 : $hash"
if ($leaks.Count -gt 0) {
    Write-Warning "빌드 경로가 남아 있다: $($leaks -join ', ') — 배포하지 말 것"
    exit 1
}
Write-Host "clean  : 빌드 머신 경로 없음" -ForegroundColor Green
