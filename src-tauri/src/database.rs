//! Application boundary: saved configuration, system credentials and native events.
mod engine;
#[cfg(test)]
mod integration_tests;
mod types;
use crate::merge::{is_cancelled, MergeFailedDto, MergeProgressDto};
use crate::model::{MergeOptions, SourceTable};
use crate::pg_import::{
    config::{self as pg_config, AppConfig, ConnectionProfile},
    credentials,
    postgres::connection::{self, ConnectionInfo},
};
use anyhow::{anyhow, Result};
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Emitter};
pub use types::{DatabaseImportFinishedDto, DatabaseImportRequest, DatabaseProfilesDto};
fn load_profiles() -> Result<(std::path::PathBuf, AppConfig)> {
    let path = pg_config::default_config_path();
    let config = pg_config::load(&path).map_err(|error| anyhow!(error.to_string()))?;
    Ok((path, config))
}

pub fn get_profiles() -> Result<DatabaseProfilesDto> {
    let (path, config) = load_profiles()?;
    Ok(DatabaseProfilesDto {
        profiles: config.profiles,
        config_path: path.display().to_string(),
    })
}

pub fn save_profile(
    name: String,
    profile: ConnectionProfile,
    password: Option<String>,
    remember_password: bool,
) -> Result<Option<String>> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("连接名称不能为空"));
    }
    validate_profile(&profile)?;
    let (path, mut config) = load_profiles()?;
    config.profiles.insert(name.to_owned(), profile);
    pg_config::save(&path, &config).map_err(|error| anyhow!(error.to_string()))?;

    let password = password.unwrap_or_default();
    let credential_warning = if remember_password && !password.is_empty() {
        credentials::store(name, &password)
            .err()
            .map(|error| format!("连接已保存，但系统凭据保存失败：{error}"))
    } else if !remember_password {
        credentials::delete(name)
            .err()
            .map(|error| format!("连接已保存，但旧的系统凭据未能删除：{error}"))
    } else {
        None
    };
    Ok(credential_warning)
}

pub fn delete_profile(name: String) -> Result<()> {
    let (path, mut config) = load_profiles()?;
    if config.profiles.remove(&name).is_none() {
        return Err(anyhow!("连接配置不存在：{name}"));
    }
    pg_config::save(&path, &config).map_err(|error| anyhow!(error.to_string()))?;
    let _ = credentials::delete(&name);
    Ok(())
}

pub async fn test_connection(
    profile_name: Option<String>,
    profile: ConnectionProfile,
    password: Option<String>,
) -> Result<ConnectionInfo> {
    validate_profile(&profile)?;
    let resolved = resolve_password(profile_name.as_deref(), password);
    let (_client, info) = connection::connect(&profile, resolved.as_deref())
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(info)
}

fn validate_profile(profile: &ConnectionProfile) -> Result<()> {
    if profile.host.trim().is_empty() {
        return Err(anyhow!("主机不能为空"));
    }
    if profile.database.trim().is_empty() {
        return Err(anyhow!("数据库不能为空"));
    }
    if profile.user.trim().is_empty() {
        return Err(anyhow!("用户名不能为空"));
    }
    if profile.port == 0 {
        return Err(anyhow!("端口必须大于 0"));
    }
    Ok(())
}

fn resolve_password(profile_name: Option<&str>, explicit: Option<String>) -> Option<String> {
    explicit
        .filter(|password| !password.is_empty())
        .or_else(|| profile_name.and_then(credentials::get))
}

pub fn spawn_database_import(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    request: DatabaseImportRequest,
    app: AppHandle,
    cancel: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        match import_database(tables, options, request, app.clone(), cancel).await {
            Ok(Some(result)) => {
                let _ = app.emit("database-import-finished", result);
            }
            Ok(None) => {
                let _ = app.emit("merge-cancelled", ());
            }
            Err(error) if is_cancelled(&error) => {
                let _ = app.emit("merge-cancelled", ());
            }
            Err(error) => {
                let _ = app.emit(
                    "merge-failed",
                    MergeFailedDto {
                        message: format!("{error:#}"),
                    },
                );
            }
        }
    });
}

async fn import_database(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    request: DatabaseImportRequest,
    app: AppHandle,
    cancel: Arc<AtomicBool>,
) -> Result<Option<DatabaseImportFinishedDto>> {
    let (_, config) = load_profiles()?;
    let profile = config
        .profiles
        .get(request.profile_name.trim())
        .cloned()
        .ok_or_else(|| anyhow!("连接配置不存在：{}", request.profile_name))?;
    validate_profile(&profile)?;

    let password = resolve_password(Some(&request.profile_name), request.password.clone());
    if request.remember_password {
        if let Some(password) = request
            .password
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            if let Err(error) = credentials::store(&request.profile_name, password) {
                crate::config::append_log(&format!("系统凭据保存失败：{error}"));
            }
        }
    }

    let emit = |current, total, label| {
        let _ = app.emit(
            "merge-progress",
            MergeProgressDto {
                current,
                total,
                label,
            },
        );
    };
    engine::import_database(tables, options, request, profile, password, cancel, &emit).await
}

#[cfg(test)]
mod tests {
    use super::engine::{parse_if_exists, parse_persistence, CopyFormat};
    use super::*;
    use crate::pg_import::postgres::table::{IfExists, TablePersistence};

    #[test]
    fn parses_database_import_options() {
        assert!(matches!(
            parse_if_exists("replace").unwrap(),
            IfExists::Replace
        ));
        assert!(matches!(
            parse_persistence("unlogged").unwrap(),
            TablePersistence::Unlogged
        ));
        assert_eq!(CopyFormat::parse("binary").unwrap().sql_name(), "binary");
    }

    #[test]
    fn rejects_incomplete_profiles() {
        let profile = ConnectionProfile {
            host: String::new(),
            ..ConnectionProfile::default()
        };
        assert!(validate_profile(&profile).is_err());
    }
}
