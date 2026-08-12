$ErrorActionPreference = "Stop"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "未检测到 Rust。请先安装：https://rustup.rs/" -ForegroundColor Yellow
    exit 1
}

Write-Host "运行测试…" -ForegroundColor Cyan
cargo test

Write-Host "构建精简 Release 版本…" -ForegroundColor Cyan
cargo build --release

$dist = Join-Path $PSScriptRoot "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$exe = Join-Path $dist "SheetForge.exe"
Copy-Item (Join-Path $PSScriptRoot "target\release\SheetForge.exe") $exe -Force

Write-Host "完成：$exe" -ForegroundColor Green
