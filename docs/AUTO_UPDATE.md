# 自动更新

SheetForge 从 `https://jp.kun.show:8443/sheetforge/latest.json` 检查便携版更新。清单包含版本、EXE 地址、SHA256 和文件大小；客户端只接受同一更新源下的 EXE，并在替换前校验大小和 SHA256。

NB 上的 `sheetforge-mirror.timer` 每 5 分钟读取 GitHub 最新 Release，将 `SheetForge.exe` 和 `SHA256SUMS.txt` 校验后镜像到 `/srv/sheetforge-updates`。Nginx 只读提供清单和版本文件，证书由 Certbot 自动续期。

Windows 客户端下载完成后启动独立 PowerShell 更新脚本，退出当前进程、替换原 EXE、启动新版；新版启动失败时恢复旧文件。便携版所在目录必须可写。
