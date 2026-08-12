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
$package = Join-Path $dist "SheetForge-Windows-x64"
New-Item -ItemType Directory -Force -Path $package | Out-Null
Copy-Item (Join-Path $PSScriptRoot "target\release\SheetForge.exe") $package -Force
Copy-Item (Join-Path $PSScriptRoot "README.md") $package -Force
Copy-Item (Join-Path $PSScriptRoot "LICENSE") $package -Force

$zip = Join-Path $dist "SheetForge-Windows-x64.zip"
if (Test-Path $zip) {
    Remove-Item $zip -Force
}
Compress-Archive -Path (Join-Path $package "*") -DestinationPath $zip -CompressionLevel Optimal

Write-Host "完成：$zip" -ForegroundColor Green

