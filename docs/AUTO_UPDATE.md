# 自动更新

SheetForge 从 `https://jp.kun.show:8443/sheetforge/latest.json` 检查便携版更新。清单包含版本、EXE 地址、SHA256 和文件大小；客户端只接受同一更新源下的 EXE，并在替换前校验大小和 SHA256。

NB 上的 `sheetforge-mirror.timer` 每 5 分钟读取 GitHub 最新 Release，将 `SheetForge.exe` 和 `SHA256SUMS.txt` 校验后镜像到 `/srv/sheetforge-updates`。Nginx 只读提供清单和版本文件，证书由 Certbot 自动续期。

Windows 客户端下载并校验完成后，通过 [self-replace](https://github.com/mitsuhiko/self-replace)（Apache-2.0，1.5.0）替换正在运行的 EXE，再使用 Rust 原生进程接口启动新版，不依赖 PowerShell 脚本。self_update 和 uv 也使用该库处理可执行文件替换。

替换前保存旧版本，新进程启动并持续运行 2 秒后才关闭当前软件；若替换失败、启动失败或新进程立即退出，则恢复旧版本并在当前窗口显示错误。便携版所在目录必须可写。自动测试在中文、空格、单引号和方括号目录中实际执行替换、重启，以及启动失败和提前退出时的恢复。

v0.7.6/v0.7.7 的旧版更新器存在脚本执行问题，迁移到 v0.7.8 需要手动下载并替换一次 EXE。
