//! PostgreSQL adapter for the shared processed row stream; no application state or credentials.
use super::types::{DatabaseImportFinishedDto, DatabaseImportRequest};
use crate::merge::stream_merged_cells;
use crate::model::{build_output_plan, MergeOptions, SourceTable};
use crate::pg_import::{
    config::ConnectionProfile,
    postgres::{
        connection,
        table::{current_persistence, prepare_table, IfExists, TablePersistence, TargetTable},
    },
    schema::identifier::{map_identifiers, quote_identifier},
    transform::{
        copy_binary_encoder::{
            encode_binary_cells_row, encode_binary_header, encode_binary_trailer,
        },
        copy_encoder::encode_copy_cells_row_into,
    },
};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::SinkExt;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
const COPY_BATCH_BYTES: usize = 8 * 1024 * 1024;
const POSTGRES_MAX_COLUMNS: usize = 1_600;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyFormat {
    Binary,
    Text,
}

impl CopyFormat {
    pub(super) fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "binary" | "二进制" => Ok(Self::Binary),
            "text" | "文本" => Ok(Self::Text),
            other => Err(anyhow!("不支持的 COPY 格式：{other}")),
        }
    }

    pub(super) fn sql_name(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Text => "text",
        }
    }

    pub(super) fn display_name(self) -> &'static str {
        match self {
            Self::Binary => "PostgreSQL 二进制",
            Self::Text => "PostgreSQL 文本",
        }
    }
}

struct CopyBatch {
    payload: Bytes,
    /// 发送该批次后累计编码的行数。
    rows: u64,
}

struct CopyProducerResult {
    merged_rows: Option<u64>,
    bytes: u64,
    batches: u64,
}

pub(super) fn parse_if_exists(value: &str) -> Result<IfExists> {
    match value.trim().to_ascii_lowercase().as_str() {
        "abort" => Ok(IfExists::Abort),
        "append" => Ok(IfExists::Append),
        "truncate" => Ok(IfExists::Truncate),
        "replace" => Ok(IfExists::Replace),
        other => Err(anyhow!("不支持的目标表策略：{other}")),
    }
}

pub(super) fn parse_persistence(value: &str) -> Result<TablePersistence> {
    match value.trim().to_ascii_lowercase().as_str() {
        "logged" | "持久化" => Ok(TablePersistence::Logged),
        "unlogged" | "极速非持久" => Ok(TablePersistence::Unlogged),
        other => Err(anyhow!("不支持的数据表持久性：{other}")),
    }
}

pub(super) async fn import_database(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    request: DatabaseImportRequest,
    profile: ConnectionProfile,
    password: Option<String>,
    cancel: Arc<AtomicBool>,
    emit: &(dyn Fn(u64, u64, String) + Send + Sync),
) -> Result<Option<DatabaseImportFinishedDto>> {
    let started = Instant::now();
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
    let estimated_rows = tables
        .iter()
        .filter(|table| table.enabled)
        .map(|table| table.estimated_rows)
        .sum::<u64>();

    // 按批次而不是按行跨线程传递。之前每一行都要经过一次 channel，导致
    // 6 百万行数据产生数百万次同步和 Vec 所有权转移；现在生产线程直接
    // 编码成 8 MiB COPY 批次，数据库线程只负责发送网络数据。
    let (batch_sender, mut batch_receiver) = tokio::sync::mpsc::channel(4);
    let producer_cancel = cancel.clone();
    let producer = tokio::task::spawn_blocking(move || -> Result<CopyProducerResult> {
        let mut buffer = Vec::with_capacity(COPY_BATCH_BYTES + 64 * 1024);
        if copy_format == CopyFormat::Binary {
            encode_binary_header(&mut buffer);
        }
        let mut bytes = buffer.len() as u64;
        let mut rows = 0_u64;
        let mut batches = 0_u64;
        let mut send_batch = |buffer: &mut Vec<u8>, rows: u64| -> Result<()> {
            if buffer.is_empty() {
                return Ok(());
            }
            let batch = std::mem::replace(buffer, Vec::with_capacity(COPY_BATCH_BYTES + 64 * 1024));
            batch_sender
                .blocking_send(CopyBatch {
                    payload: Bytes::from(batch),
                    rows,
                })
                .map_err(|_| anyhow!("数据库写入端已停止"))?;
            batches += 1;
            Ok(())
        };

        let merged =
            stream_merged_cells(&tables, &options, &|_, _, _| {}, &producer_cancel, |row| {
                let before = buffer.len();
                match copy_format {
                    CopyFormat::Binary => {
                        encode_binary_cells_row(row, request.empty_as_null, &mut buffer)
                            .map_err(|error| anyhow!(error.to_string()))?
                    }
                    CopyFormat::Text => {
                        encode_copy_cells_row_into(row, request.empty_as_null, &mut buffer)
                    }
                }
                bytes += (buffer.len() - before) as u64;
                rows += 1;
                if buffer.len() >= COPY_BATCH_BYTES {
                    send_batch(&mut buffer, rows)?;
                }
                Ok(())
            })?;

        let merged_rows = merged.map(|(_, rows)| rows);
        if merged_rows.is_some() {
            if copy_format == CopyFormat::Binary {
                encode_binary_trailer(&mut buffer);
            }
            send_batch(&mut buffer, rows)?;
        }
        Ok(CopyProducerResult {
            merged_rows,
            bytes,
            batches,
        })
    });

    let mut rows = 0_u64;
    while let Some(batch) = batch_receiver.recv().await {
        if cancel.load(Ordering::Relaxed) {
            drop(batch_receiver);
            let _ = producer.await;
            return Ok(None);
        }
        rows = batch.rows;
        sink.as_mut().send(batch.payload).await?;

        if rows > 0 {
            emit(rows, estimated_rows, "正在导入 PostgreSQL".to_owned());
        }
    }

    // 生产线程因取消而提前结束时通道会直接关闭，循环内的取消检查不会执行；
    // 这里统一按取消处理，事务随 client 释放而回滚。
    if cancel.load(Ordering::Relaxed) {
        let _ = producer.await;
        return Ok(None);
    }
    let produced = producer
        .await
        .map_err(|error| anyhow!("表格处理线程异常：{error}"))??;
    let Some(produced_rows) = produced.merged_rows else {
        return Ok(None);
    };
    if produced_rows != rows {
        return Err(anyhow!(
            "行流计数不一致：处理端 {produced_rows} 行，数据库端 {rows} 行"
        ));
    }
    let server_rows = sink.as_mut().finish().await?;
    if server_rows != rows {
        return Err(anyhow!(
            "PostgreSQL COPY 确认行数异常：客户端发送 {rows} 行，服务器确认 {server_rows} 行"
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        transaction.rollback().await?;
        return Ok(None);
    }
    transaction.commit().await?;

    emit(rows, rows, "PostgreSQL 已提交事务".to_owned());
    Ok(Some(DatabaseImportFinishedDto {
        rows,
        bytes: produced.bytes,
        batches: produced.batches,
        server: profile.host,
        database: connection_info.database,
        target: format!("{}.{}", target.schema, target.table),
        elapsed_ms: started.elapsed().as_millis(),
        copy_format: copy_format.display_name().to_owned(),
        table_persistence: actual_persistence.chinese_name().to_owned(),
        copy_freeze,
    }))
}
