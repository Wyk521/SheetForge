use crate::merge::{stream_merged_rows, MergeFailedDto, MergeProgressDto};
use crate::model::{build_output_plan, MergeOptions, SourceTable};
use crate::pg_import::{
    config::{self as pg_config, AppConfig, ConnectionProfile},
    credentials,
    postgres::{
        connection::{self, ConnectionInfo},
        table::{current_persistence, prepare_table, IfExists, TablePersistence, TargetTable},
    },
    schema::identifier::{map_identifiers, quote_identifier},
    transform::{
        copy_binary_encoder::{encode_binary_header, encode_binary_row, encode_binary_trailer},
        copy_encoder::encode_copy_row_into,
    },
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

const COPY_BATCH_BYTES: usize = 8 * 1024 * 1024;
const POSTGRES_MAX_COLUMNS: usize = 1_600;

#[derive(Clone, Serialize)]
pub struct DatabaseProfilesDto {
    pub profiles: BTreeMap<String, ConnectionProfile>,
    pub config_path: String,
}

#[derive(Clone, Deserialize)]
pub struct DatabaseImportRequest {
    pub profile_name: String,
    pub password: Option<String>,
    pub remember_password: bool,
    pub schema: String,
    pub table: String,
    pub if_exists: String,
    pub copy_format: String,
    pub table_persistence: String,
    pub empty_as_null: bool,
    pub fast_mode: bool,
}

#[derive(Clone, Serialize)]
pub struct DatabaseImportFinishedDto {
    pub rows: u64,
    pub bytes: u64,
    pub batches: u64,
    pub server: String,
    pub database: String,
    pub target: String,
    pub elapsed_ms: u128,
    pub copy_format: String,
    pub table_persistence: String,
    pub copy_freeze: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyFormat {
    Binary,
    Text,
}

impl CopyFormat {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "binary" | "二进制" => Ok(Self::Binary),
            "text" | "文本" => Ok(Self::Text),
            other => Err(anyhow!("不支持的 COPY 格式：{other}")),
        }
    }

    fn sql_name(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Text => "text",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Binary => "PostgreSQL 二进制",
            Self::Text => "PostgreSQL 文本",
        }
    }
}

fn parse_if_exists(value: &str) -> Result<IfExists> {
    match value.trim().to_ascii_lowercase().as_str() {
        "abort" => Ok(IfExists::Abort),
        "append" => Ok(IfExists::Append),
        "truncate" => Ok(IfExists::Truncate),
        "replace" => Ok(IfExists::Replace),
        other => Err(anyhow!("不支持的目标表策略：{other}")),
    }
}

fn parse_persistence(value: &str) -> Result<TablePersistence> {
    match value.trim().to_ascii_lowercase().as_str() {
        "logged" | "持久化" => Ok(TablePersistence::Logged),
        "unlogged" | "极速非持久" => Ok(TablePersistence::Unlogged),
        other => Err(anyhow!("不支持的数据表持久性：{other}")),
    }
}

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
    let started = Instant::now();
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

    let plan = build_output_plan(&tables, &options);
    if plan.headers.is_empty() {
        return Err(anyhow!("合并后没有可导入的列，请检查合并方式或字段映射"));
    }
    if plan.headers.len() > POSTGRES_MAX_COLUMNS {
        return Err(anyhow!(
            "输出列数为 {}，超过 PostgreSQL 单表最多 1,600 列的限制",
            plan.headers.len()
        ));
    }

    let copy_format = CopyFormat::parse(&request.copy_format)?;
    let table_persistence = parse_persistence(&request.table_persistence)?;
    let target = TargetTable {
        schema: request.schema.trim().to_owned(),
        table: request.table.trim().to_owned(),
        if_exists: parse_if_exists(&request.if_exists)?,
    };
    target
        .validate()
        .map_err(|error| anyhow!(error.to_string()))?;

    let (mut client, connection_info) = connection::connect(&profile, password.as_deref())
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }

    let identifiers = map_identifiers(&plan.headers);
    let transaction = client.transaction().await?;
    if request.fast_mode {
        transaction
            .execute("SET LOCAL synchronous_commit = off", &[])
            .await?;
    }
    let copy_freeze = prepare_table(&transaction, &target, &identifiers, table_persistence)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let actual_persistence = current_persistence(&transaction, &target)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;

    let columns = identifiers
        .iter()
        .map(|column| quote_identifier(&column.database))
        .collect::<Vec<_>>()
        .join(", ");
    let freeze = if copy_freeze { ", FREEZE true" } else { "" };
    let sql = format!(
        "COPY {} ({columns}) FROM STDIN WITH (FORMAT {}{freeze})",
        target.quoted(),
        copy_format.sql_name()
    );
    let mut sink = Box::pin(transaction.copy_in(&sql).await?);
    let mut buffer = Vec::with_capacity(COPY_BATCH_BYTES + 64 * 1024);
    if copy_format == CopyFormat::Binary {
        encode_binary_header(&mut buffer);
    }
    let mut bytes = buffer.len() as u64;
    let mut rows = 0_u64;
    let mut batches = 0_u64;
    let estimated_rows = tables
        .iter()
        .filter(|table| table.enabled)
        .map(|table| table.estimated_rows)
        .sum::<u64>();

    let (row_sender, mut row_receiver) = tokio::sync::mpsc::channel(256);
    let producer_cancel = cancel.clone();
    let producer = tauri::async_runtime::spawn_blocking(move || {
        stream_merged_rows(
            &tables,
            &options,
            request.empty_as_null,
            &|_, _, _| {},
            &producer_cancel,
            |row| {
                row_sender
                    .blocking_send(row)
                    .map_err(|_| anyhow!("数据库写入端已停止"))
            },
        )
    });

    while let Some(row) = row_receiver.recv().await {
        if cancel.load(Ordering::Relaxed) {
            drop(row_receiver);
            let _ = producer.await;
            return Ok(None);
        }
        let before = buffer.len();
        match copy_format {
            CopyFormat::Binary => {
                encode_binary_row(&row, &mut buffer).map_err(|error| anyhow!(error.to_string()))?
            }
            CopyFormat::Text => encode_copy_row_into(&row, &mut buffer),
        }
        bytes += (buffer.len() - before) as u64;
        rows += 1;

        if rows.is_multiple_of(1_000) || rows == estimated_rows {
            let _ = app.emit(
                "merge-progress",
                MergeProgressDto {
                    current: rows,
                    total: estimated_rows,
                    label: "正在导入 PostgreSQL".to_owned(),
                },
            );
        }
        if buffer.len() >= COPY_BATCH_BYTES {
            let batch = std::mem::replace(
                &mut buffer,
                Vec::with_capacity(COPY_BATCH_BYTES + 64 * 1024),
            );
            sink.as_mut().send(Bytes::from(batch)).await?;
            batches += 1;
        }
    }

    let produced = producer
        .await
        .map_err(|error| anyhow!("表格处理线程异常：{error}"))??;
    let Some((_produced_plan, produced_rows)) = produced else {
        return Ok(None);
    };
    if produced_rows != rows {
        return Err(anyhow!(
            "行流计数不一致：处理端 {produced_rows} 行，数据库端 {rows} 行"
        ));
    }

    if copy_format == CopyFormat::Binary {
        let before = buffer.len();
        encode_binary_trailer(&mut buffer);
        bytes += (buffer.len() - before) as u64;
    }
    if !buffer.is_empty() {
        sink.as_mut().send(Bytes::from(buffer)).await?;
        batches += 1;
    }
    let server_rows = sink.as_mut().finish().await?;
    if server_rows != rows {
        return Err(anyhow!(
            "PostgreSQL COPY 确认行数异常：客户端发送 {rows} 行，服务器确认 {server_rows} 行"
        ));
    }
    transaction.commit().await?;

    let _ = app.emit(
        "merge-progress",
        MergeProgressDto {
            current: rows,
            total: rows,
            label: "PostgreSQL 已提交事务".to_owned(),
        },
    );
    Ok(Some(DatabaseImportFinishedDto {
        rows,
        bytes,
        batches,
        server: profile.host,
        database: connection_info.database,
        target: format!("{}.{}", target.schema, target.table),
        elapsed_ms: started.elapsed().as_millis(),
        copy_format: copy_format.display_name().to_owned(),
        table_persistence: actual_persistence.chinese_name().to_owned(),
        copy_freeze,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

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
