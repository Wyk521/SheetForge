use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
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

    let _new_process = replace_and_launch(&current_exe, &staged_exe)?;

    let _ = app.emit("update-installing", ());
    app.exit(0);
    Ok(())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn replace_and_launch(current_exe: &Path, staged_exe: &Path) -> Result<std::process::Child> {
    let directory = current_exe.parent().context("当前程序目录无效")?;
    if !staged_exe.is_file() {
        bail!("更新文件不存在，软件未退出");
    }
    let backup = tempfile::Builder::new()
        .prefix(".SheetForge-rollback-")
        .tempfile_in(directory)
        .context("无法保存旧版本，软件未退出")?;
    fs::copy(current_exe, backup.path()).context("无法保存旧版本，软件未退出")?;

    // Reuse the same native EXE replacement library used by self_update and uv.
    if let Err(error) = self_replace::self_replace(staged_exe) {
        if !current_exe.exists() {
            restore_update_backup(backup, current_exe)?;
        }
        return Err(error).context("替换更新失败，软件未退出");
    }

    match launch_replacement(current_exe) {
        Ok(child) => {
            let _ = fs::remove_file(staged_exe);
            Ok(child)
        }
        Err(error) => {
            restore_update_backup(backup, current_exe)?;
            Err(error).context("新版启动失败，已恢复旧版本，软件未退出")
        }
    }
}

fn restore_update_backup(backup: tempfile::NamedTempFile, current_exe: &Path) -> Result<()> {
    if let Err(error) = fs::rename(backup.path(), current_exe) {
        let (_, preserved_path) = backup.keep().context("无法保留旧版本备份")?;
        bail!(
            "恢复旧版本失败：{error}。备份保存在 {}，当前软件仍在运行",
            preserved_path.display()
        );
    }
    Ok(())
}

fn launch_replacement(current_exe: &Path) -> Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(current_exe);
    command
        .args(std::env::args_os().skip(1))
        .current_dir(current_exe.parent().context("当前程序目录无效")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().context("无法启动新版程序")?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    match child.try_wait() {
        Ok(None) => Ok(child),
        Ok(Some(status)) => bail!("新版程序启动后退出（{status}）"),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error).context("无法确认新版程序状态")
        }
    }
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
    fn missing_update_does_not_touch_current_executable() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("current.exe");
        fs::write(&current, b"original").unwrap();
        let result = replace_and_launch(&current, &dir.path().join("missing.exe"));
        assert!(result.is_err());
        assert_eq!(fs::read(current).unwrap(), b"original");
    }

    // Run the updater in a disposable copy of the test executable, never the real test runner.
    #[test]
    fn native_update_child_entry() {
        let Some(staged) = std::env::var_os("SHEETFORGE_TEST_UPDATE_STAGE") else {
            return;
        };
        let current = std::env::current_exe().unwrap();
        let result = replace_and_launch(&current, Path::new(&staged));
        if std::env::var_os("SHEETFORGE_TEST_EXPECT_FAILURE").is_some() {
            assert!(result.is_err(), "a broken update must not be accepted");
        } else {
            result.unwrap().wait().unwrap();
        }
    }

    fn run_native_update_fixture(body: Option<&str>, expect_failure: bool) {
        use std::process::Command;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("中文 空格's [便携版]");
        fs::create_dir(&directory).unwrap();
        let current = directory.join("SheetForge.exe");
        let staged = directory.join(".SheetForge.exe.update");
        fs::copy(std::env::current_exe().unwrap(), &current).unwrap();
        let original = fs::read(&current).unwrap();
        if let Some(body) = body {
            let source = directory.join("stub.rs");
            fs::write(&source, body).unwrap();
            assert!(Command::new("rustc")
                .arg("--crate-name=update_stub")
                .arg(&source)
                .arg("-o")
                .arg(&staged)
                .status()
                .unwrap()
                .success());
        } else {
            fs::write(&staged, b"not an executable").unwrap();
        }
        let updated = fs::read(&staged).unwrap();
        let mut command = Command::new(&current);
        command
            .args([
                "--exact",
                "updater::tests::native_update_child_entry",
                "--nocapture",
            ])
            .env("SHEETFORGE_TEST_UPDATE_STAGE", &staged);
        if expect_failure {
            command.env("SHEETFORGE_TEST_EXPECT_FAILURE", "1");
        } else {
            command.env_remove("SHEETFORGE_TEST_EXPECT_FAILURE");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "native update failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if expect_failure {
            assert_eq!(fs::read(&current).unwrap(), original);
            assert!(staged.exists(), "keep the download when restarting fails");
        } else {
            assert_eq!(fs::read(&current).unwrap(), updated);
            assert_eq!(
                fs::read_to_string(directory.join("launched.txt")).unwrap(),
                "new"
            );
            assert!(!staged.exists());
        }
        assert!(!fs::read_dir(&directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".SheetForge-rollback-")
        }));
    }

    #[test]
    fn native_update_replaces_and_restarts_in_unicode_directory() {
        run_native_update_fixture(
            Some(
                r#"fn main() {
                std::fs::write("launched.txt", "new").unwrap();
                std::thread::sleep(std::time::Duration::from_secs(4));
            }"#,
            ),
            false,
        );
    }

    #[test]
    fn native_update_restores_old_version_if_new_executable_is_invalid() {
        run_native_update_fixture(None, true);
    }

    #[test]
    fn native_update_restores_old_version_if_new_process_exits_early() {
        run_native_update_fixture(Some("fn main() {}"), true);
    }
}
