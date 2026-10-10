use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

const UPDATE_MANIFEST_URL: &str = "https://jp.kun.show:8443/sheetforge/latest.json";
const UPDATE_DOWNLOAD_PREFIX: &str = "https://jp.kun.show:8443/sheetforge/releases/";
const MAX_UPDATE_SIZE: u64 = 256 * 1024 * 1024;

static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateManifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Serialize)]
struct UpdateResultDto {
    version: String,
    url: String,
    sha256: String,
    size: u64,
    newer: bool,
}

#[derive(Clone, Serialize)]
struct UpdateProgressDto {
    downloaded: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct UpdateFailedDto {
    message: String,
}

#[tauri::command]
pub fn check_update(app: AppHandle) {
    std::thread::spawn(move || {
        let result = fetch_manifest();
        match result {
            Ok(update) => {
                let newer = is_newer_version(&update.version, env!("CARGO_PKG_VERSION"));
                let _ = app.emit(
                    "update-result",
                    UpdateResultDto {
                        version: update.version,
                        url: update.url,
                        sha256: update.sha256,
                        size: update.size,
                        newer,
                    },
                );
            }
            Err(error) => emit_failure(&app, error),
        }
    });
}

#[tauri::command]
pub fn install_update(app: AppHandle, update: UpdateManifest) -> Result<(), String> {
    validate_manifest(&update).map_err(|error| format!("{error:#}"))?;
    if !is_newer_version(&update.version, env!("CARGO_PKG_VERSION")) {
        return Err("更新版本无效".to_owned());
    }
    if UPDATE_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("更新正在进行".to_owned());
    }

    std::thread::spawn(move || {
        if let Err(error) = download_and_restart(&app, &update) {
            UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
            emit_failure(&app, error);
        }
    });
    Ok(())
}

fn fetch_manifest() -> Result<UpdateManifest> {
    let user_agent = format!("SheetForge/{}", env!("CARGO_PKG_VERSION"));
    let body = ureq::get(UPDATE_MANIFEST_URL)
        .header("User-Agent", &user_agent)
        .call()
        .context("无法连接更新服务器")?
        .body_mut()
        .read_to_string()
        .context("无法读取更新信息")?;
    let update: UpdateManifest = serde_json::from_str(&body).context("更新信息格式不正确")?;
    validate_manifest(&update)?;
    Ok(update)
}

fn validate_manifest(update: &UpdateManifest) -> Result<()> {
    if version_tuple(&update.version).is_none() {
        bail!("更新版本号无效");
    }
    if !update.url.starts_with(UPDATE_DOWNLOAD_PREFIX) || !update.url.ends_with(".exe") {
        bail!("更新下载地址无效");
    }
    if update.sha256.len() != 64 || !update.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("更新校验值无效");
    }
    if update.size == 0 || update.size > MAX_UPDATE_SIZE {
        bail!("更新文件大小无效");
    }
    Ok(())
}

fn download_and_restart(app: &AppHandle, update: &UpdateManifest) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, update);
        bail!("自动更新仅支持 Windows 便携版");
    }

    #[cfg(target_os = "windows")]
    {
        download_and_restart_portable(app, update)
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn download_and_restart_portable(app: &AppHandle, update: &UpdateManifest) -> Result<()> {
    use sha2::{Digest, Sha256};

    let current_exe = std::env::current_exe().context("无法确定当前程序路径")?;
    let file_name = current_exe
        .file_name()
        .and_then(|value| value.to_str())
        .context("当前程序文件名无效")?;
    let staged_exe = current_exe.with_file_name(format!(".{file_name}.{}.update", update.version));
    if staged_exe.exists() {
        fs::remove_file(&staged_exe).context("无法清理上次未完成的更新")?;
    }

    let user_agent = format!("SheetForge/{}", env!("CARGO_PKG_VERSION"));
    let mut response = ureq::get(&update.url)
        .header("User-Agent", &user_agent)
        .call()
        .context("下载更新失败")?;
    let mut reader = response.body_mut().as_reader();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_exe)
        .context("当前目录不可写，无法更新便携版")?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    let download_result = (|| -> Result<()> {
        loop {
            let read = reader.read(&mut buffer).context("读取更新数据失败")?;
            if read == 0 {
                break;
            }
            downloaded += read as u64;
            if downloaded > update.size || downloaded > MAX_UPDATE_SIZE {
                bail!("更新文件大小不一致");
            }
            output
                .write_all(&buffer[..read])
                .context("写入更新文件失败")?;
            hasher.update(&buffer[..read]);
            let _ = app.emit(
                "update-download-progress",
                UpdateProgressDto {
                    downloaded,
                    total: update.size,
                },
            );
        }
        output.flush().context("保存更新文件失败")?;
        output.sync_all().context("保存更新文件失败")?;
        if downloaded != update.size {
            bail!("更新文件大小不一致");
        }
        let actual_hash = hex::encode(hasher.finalize());
        if !actual_hash.eq_ignore_ascii_case(&update.sha256) {
            bail!("更新文件校验失败");
        }
        Ok(())
    })();

    if let Err(error) = download_result {
        drop(output);
        let _ = fs::remove_file(&staged_exe);
        return Err(error);
    }
    drop(output);

    let script_path = updater_script_path();
    let script = windows_update_script(&current_exe, &staged_exe, std::process::id());
    write_update_script(&script_path, &script)?;

    start_update_script(&script_path)?;

    let _ = app.emit("update-installing", ());
    app.exit(0);
    Ok(())
}

fn updater_script_path() -> PathBuf {
    std::env::temp_dir().join(format!("SheetForge-update-{}.ps1", std::process::id()))
}

fn powershell_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn write_update_script(path: &Path, script: &str) -> Result<()> {
    // powershell.exe is Windows PowerShell 5.1, which otherwise reads UTF-8 as ANSI.
    let mut encoded = Vec::with_capacity(script.len() + 3);
    encoded.extend_from_slice(b"\xef\xbb\xbf");
    encoded.extend_from_slice(script.as_bytes());
    fs::write(path, encoded).context("无法创建更新脚本")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn start_update_script(script_path: &Path) -> Result<()> {
    let ready_path = script_path.with_extension("ready");
    if ready_path.exists() {
        fs::remove_file(&ready_path).context("无法清理更新启动状态")?;
    }
    let mut child = spawn_update_script(script_path)?;
    let result =
        wait_for_update_script(&mut child, &ready_path, std::time::Duration::from_secs(15));
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_file(script_path);
    }
    let _ = fs::remove_file(&ready_path);
    result
}

fn wait_for_update_script(
    child: &mut std::process::Child,
    ready_path: &Path,
    timeout: std::time::Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().context("无法检查更新程序状态")? {
            bail!("更新程序启动失败（{status}），请查看临时目录中的 SheetForge-update.log");
        }
        if ready_path.is_file() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            bail!("更新程序启动超时，软件未退出，请查看临时目录中的 SheetForge-update.log");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn windows_update_script(current_exe: &Path, staged_exe: &Path, process_id: u32) -> String {
    let current = powershell_literal(current_exe);
    let staged = powershell_literal(staged_exe);
    format!(
        r#"$ErrorActionPreference = 'Stop'
$processId = {process_id}
$currentExe = '{current}'
$stagedExe = '{staged}'
$replacement = "$currentExe.new"
$backup = "$currentExe.old"
$ready = [IO.Path]::ChangeExtension($PSCommandPath, 'ready')
$workingDirectory = Split-Path -LiteralPath $currentExe
$started = $false
try {{
    if (-not (Test-Path -LiteralPath $stagedExe -PathType Leaf)) {{ throw 'Update file is missing' }}
    $oldProcess = Get-Process -Id $processId -ErrorAction SilentlyContinue
    [IO.File]::WriteAllText($ready, 'ready')
    if ($null -ne $oldProcess) {{ $oldProcess | Wait-Process -Timeout 60 -ErrorAction Stop }}
    Copy-Item -LiteralPath $stagedExe -Destination $replacement -Force
    if (Test-Path -LiteralPath $backup) {{ Remove-Item -LiteralPath $backup -Force }}
    for ($attempt = 0; ; $attempt++) {{
        try {{
            Move-Item -LiteralPath $currentExe -Destination $backup -Force
            break
        }} catch {{
            if ($attempt -ge 49) {{ throw }}
            Start-Sleep -Milliseconds 100
        }}
    }}
    try {{
        Move-Item -LiteralPath $replacement -Destination $currentExe -Force
    }} catch {{
        Move-Item -LiteralPath $backup -Destination $currentExe -Force
        throw
    }}
    try {{
        $newProcess = Start-Process -FilePath $currentExe -WorkingDirectory $workingDirectory -PassThru
        $started = $true
    }} catch {{
        Remove-Item -LiteralPath $currentExe -Force -ErrorAction SilentlyContinue
        Move-Item -LiteralPath $backup -Destination $currentExe -Force
        Start-Process -FilePath $currentExe -WorkingDirectory $workingDirectory
        $started = $true
        throw
    }}
    Start-Sleep -Seconds 2
    $newProcess.Refresh()
    if ($newProcess.HasExited) {{
        Remove-Item -LiteralPath $currentExe -Force -ErrorAction SilentlyContinue
        Move-Item -LiteralPath $backup -Destination $currentExe -Force
        Start-Process -FilePath $currentExe -WorkingDirectory $workingDirectory
        throw '新版本启动失败，已恢复旧版本'
    }}
    if (Test-Path -LiteralPath $backup) {{ Remove-Item -LiteralPath $backup -Force }}
    if (Test-Path -LiteralPath $stagedExe) {{ Remove-Item -LiteralPath $stagedExe -Force }}
}} catch {{
    Add-Content -LiteralPath "$env:TEMP\SheetForge-update.log" -Encoding UTF8 -Value "$(Get-Date -Format o) $($_.Exception.Message)"
    if (-not $started -and $null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue) -and (Test-Path -LiteralPath $currentExe)) {{
        Start-Process -FilePath $currentExe -WorkingDirectory $workingDirectory
    }}
    exit 1
}} finally {{
    Remove-Item -LiteralPath $ready -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $replacement) {{ Remove-Item -LiteralPath $replacement -Force -ErrorAction SilentlyContinue }}
    Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
}}
"#
    )
}

#[cfg(target_os = "windows")]
fn spawn_update_script(script_path: &Path) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("SheetForge-update.log"))
        .context("无法创建更新日志")?;
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script_path)
        .stdin(Stdio::null())
        .stdout(log.try_clone().context("无法打开更新日志")?)
        .stderr(log)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("无法启动更新程序")
}

#[cfg(not(target_os = "windows"))]
fn spawn_update_script(_script_path: &Path) -> Result<std::process::Child> {
    bail!("自动更新仅支持 Windows 便携版")
}

fn emit_failure(app: &AppHandle, error: anyhow::Error) {
    let _ = app.emit(
        "update-failed",
        UpdateFailedDto {
            message: format!("{error:#}"),
        },
    );
}

fn version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let trimmed = version.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn is_newer_version(remote: &str, current: &str) -> bool {
    match (version_tuple(remote), version_tuple(current)) {
        (Some(remote), Some(current)) => remote > current,
        (Some(_), None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_is_semantic() {
        assert!(is_newer_version("v0.10.0", "0.3.1"));
        assert!(!is_newer_version("v0.3.1", "0.3.1"));
        assert!(!is_newer_version("v0.2.9", "0.3.1"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(!is_newer_version("junk", "0.3.1"));
        assert!(!is_newer_version("1.2.3.4", "1.2.3"));
    }

    #[test]
    fn manifest_rejects_untrusted_downloads() {
        let valid = UpdateManifest {
            version: "0.8.0".to_owned(),
            url: format!("{UPDATE_DOWNLOAD_PREFIX}0.8.0/SheetForge.exe"),
            sha256: "a".repeat(64),
            size: 1024,
        };
        assert!(validate_manifest(&valid).is_ok());

        let mut untrusted = valid.clone();
        untrusted.url = "https://example.com/SheetForge.exe".to_owned();
        assert!(validate_manifest(&untrusted).is_err());
    }

    #[test]
    fn update_script_waits_replaces_and_restarts() {
        let script = windows_update_script(
            Path::new(r"C:\工具\SheetForge.exe"),
            Path::new(r"C:\工具\.SheetForge.update"),
            42,
        );
        assert!(script.contains("$oldProcess | Wait-Process -Timeout 60 -ErrorAction Stop"));
        assert!(script.contains("Move-Item -LiteralPath $replacement -Destination $currentExe"));
        assert!(script.contains("Start-Process -FilePath $currentExe"));
    }

    #[test]
    fn update_script_preserves_unicode_and_quotes_with_utf8_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.ps1");
        let script = windows_update_script(
            Path::new(r"C:\工具箱\用户's [便携版]\SheetForge.exe"),
            Path::new(r"C:\工具箱\用户's [便携版]\.SheetForge.exe.update"),
            42,
        );
        write_update_script(&path, &script).unwrap();
        let bytes = fs::read(path).unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(std::str::from_utf8(&bytes[3..]).unwrap(), script);
        assert!(script.contains(r"用户''s [便携版]"));
    }

    #[test]
    fn exited_helper_is_not_treated_as_ready() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("update.ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        // Even a stale marker must not make a dead helper look ready.
        fs::write(&ready, "ready").unwrap();
        let result = wait_for_update_script(&mut child, &ready, std::time::Duration::from_secs(1));
        assert!(result.unwrap_err().to_string().contains("更新程序启动失败"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_script_parse_failure_prevents_update_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broken.ps1");
        write_update_script(&script, "try { this script is incomplete").unwrap();
        let error = start_update_script(&script).unwrap_err();
        assert!(error.to_string().contains("更新程序启动失败"));
        assert!(!script.exists());
        assert!(!script.with_extension("ready").exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_powershell_replaces_and_restarts_in_unicode_directory() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("中文 空格's [便携版]");
        fs::create_dir(&dir).unwrap();
        let current = dir.join("SheetForge.exe");
        let staged = dir.join(".SheetForge.exe.update");
        let source = dir.join("stub.rs");
        fs::write(
            &source,
            r#"fn main() {
                std::fs::write("launched.txt", "new").unwrap();
                std::thread::sleep(std::time::Duration::from_secs(4));
            }"#,
        )
        .unwrap();
        assert!(Command::new("rustc")
            .arg("--crate-name=update_stub")
            .arg(&source)
            .arg("-o")
            .arg(&staged)
            .status()
            .unwrap()
            .success());
        let new_bytes = fs::read(&staged).unwrap();
        fs::write(&current, b"old executable").unwrap();
        let mut old_process = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let script_path = dir.join("update.ps1");
        write_update_script(
            &script_path,
            &windows_update_script(&current, &staged, old_process.id()),
        )
        .unwrap();
        let mut helper = spawn_update_script(&script_path).unwrap();
        let ready = wait_for_update_script(
            &mut helper,
            &script_path.with_extension("ready"),
            Duration::from_secs(15),
        );
        let untouched = fs::read(&current).unwrap() == b"old executable";
        old_process.kill().unwrap();
        old_process.wait().unwrap();
        ready.unwrap();
        assert!(
            untouched,
            "replacement must wait for the old process to exit"
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = helper.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                helper.kill().unwrap();
                helper.wait().unwrap();
                panic!("update helper did not finish");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(
            status.success(),
            "check SheetForge-update.log for helper errors"
        );
        assert_eq!(fs::read_to_string(dir.join("launched.txt")).unwrap(), "new");
        assert_eq!(fs::read(&current).unwrap(), new_bytes);
        assert!(!staged.exists());
        assert!(!script_path.exists());
        assert!(!script_path.with_extension("ready").exists());
        assert!(!dir.join("SheetForge.exe.old").exists());
        std::thread::sleep(Duration::from_secs(3));
    }
}
