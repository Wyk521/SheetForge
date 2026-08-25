use std::collections::HashSet;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, Transaction};

use crate::pg_import::{
    schema::identifier::{quote_identifier, IdentifierMapping, POSTGRES_IDENTIFIER_MAX_BYTES},
    AppError, Result,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum IfExists {
    #[default]
    Abort,
    Append,
    Truncate,
    Replace,
}

impl IfExists {
    #[must_use]
    pub const fn chinese_name(self) -> &'static str {
        match self {
            Self::Abort => "表已存在则停止",
            Self::Append => "追加数据",
            Self::Truncate => "清空后导入",
            Self::Replace => "重建数据表",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum TablePersistence {
    #[default]
    #[value(name = "持久化", alias = "logged")]
    Logged,
    #[value(name = "极速非持久", alias = "unlogged")]
    Unlogged,
}

impl TablePersistence {
    #[must_use]
    pub const fn chinese_name(self) -> &'static str {
        match self {
            Self::Logged => "标准持久化（崩溃可恢复）",
            Self::Unlogged => "极速非持久（UNLOGGED）",
        }
    }

    const fn create_keyword(self) -> &'static str {
        match self {
            Self::Logged => "",
            Self::Unlogged => "UNLOGGED ",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TargetTable {
    pub schema: String,
    pub table: String,
    pub if_exists: IfExists,
}

impl TargetTable {
    pub fn validate(&self) -> Result<()> {
        for (kind, value) in [("schema", &self.schema), ("table", &self.table)] {
            if value.is_empty() {
                return Err(AppError::Config(format!("{kind} 不能为空")));
            }
            if value.len() > POSTGRES_IDENTIFIER_MAX_BYTES {
                return Err(AppError::Config(format!(
                    "{kind} 名称超过 PostgreSQL 63 字节限制: {value}"
                )));
            }
            if value.as_bytes().contains(&0) {
                return Err(AppError::Config(format!("{kind} 不能包含 NUL")));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn quoted(&self) -> String {
        format!(
            "{}.{}",
            quote_identifier(&self.schema),
            quote_identifier(&self.table)
        )
    }
}

async fn schema_exists(transaction: &Transaction<'_>, schema: &str) -> Result<bool> {
    Ok(transaction
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
            &[&schema],
        )
        .await?
        .get(0))
}

pub async fn table_exists(transaction: &Transaction<'_>, target: &TargetTable) -> Result<bool> {
    Ok(transaction
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = $1 AND c.relname = $2
                  AND c.relkind IN ('r', 'p', 'f')
            )",
            &[&target.schema, &target.table],
        )
        .await?
        .get(0))
}

pub async fn target_exists(client: &Client, target: &TargetTable) -> Result<bool> {
    target.validate()?;
    Ok(client
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = $1 AND c.relname = $2
                  AND c.relkind IN ('r', 'p', 'f')
            )",
            &[&target.schema, &target.table],
        )
        .await?
        .get(0))
}

pub async fn current_persistence(
    transaction: &Transaction<'_>,
    target: &TargetTable,
) -> Result<TablePersistence> {
    let unlogged: bool = transaction
        .query_one(
            "SELECT c.relpersistence = 'u'
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = $2
               AND c.relkind IN ('r', 'p')",
            &[&target.schema, &target.table],
        )
        .await?
        .get(0);
    Ok(if unlogged {
        TablePersistence::Unlogged
    } else {
        TablePersistence::Logged
    })
}
async fn create_table(
    transaction: &Transaction<'_>,
    target: &TargetTable,
    columns: &[IdentifierMapping],
    persistence: TablePersistence,
) -> Result<()> {
    if columns.is_empty() {
        return Err(AppError::Config("无法创建零字段表".into()));
    }
    let definitions = columns
        .iter()
        .map(|column| format!("{} TEXT NULL", quote_identifier(&column.database)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "CREATE {}TABLE {} ({definitions})",
        persistence.create_keyword(),
        target.quoted()
    );
    transaction.execute(&sql, &[]).await?;
    Ok(())
}

async fn validate_append(
    transaction: &Transaction<'_>,
    target: &TargetTable,
    imported: &[IdentifierMapping],
) -> Result<()> {
    let rows = transaction
        .query(
            "SELECT a.attname, t.typname, a.attnotnull, d.oid IS NOT NULL
             FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_type t ON t.oid = a.atttypid
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             WHERE n.nspname = $1 AND c.relname = $2
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
            &[&target.schema, &target.table],
        )
        .await?;

    let imported_names: HashSet<&str> = imported
        .iter()
        .map(|column| column.database.as_str())
        .collect();
    let mut existing_names = HashSet::new();
    let mut errors = Vec::new();
    for row in rows {
        let name: String = row.get(0);
        let type_name: String = row.get(1);
        let not_null: bool = row.get(2);
        let has_default: bool = row.get(3);
        existing_names.insert(name.clone());

        if imported_names.contains(name.as_str())
            && !matches!(type_name.as_str(), "text" | "varchar" | "bpchar")
        {
            errors.push(format!("目标字段 {name} 类型 {type_name} 与 TEXT 不兼容"));
        }
        if !imported_names.contains(name.as_str()) && not_null && !has_default {
            errors.push(format!("额外字段 {name} 为 NOT NULL 且没有默认值"));
        }
    }

    for imported_name in imported_names {
        if !existing_names.contains(imported_name) {
            errors.push(format!("目标表缺少字段 {imported_name}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Config(format!(
            "append 前置检查失败: {}",
            errors.join("；")
        )))
    }
}

pub async fn prepare_table(
    transaction: &Transaction<'_>,
    target: &TargetTable,
    columns: &[IdentifierMapping],
    persistence: TablePersistence,
) -> Result<bool> {
    target.validate()?;
    if !schema_exists(transaction, &target.schema).await? {
        return Err(AppError::Config(format!("schema {} 不存在", target.schema)));
    }
    let exists = table_exists(transaction, target).await?;
    match (exists, target.if_exists) {
        (false, _) => {
            create_table(transaction, target, columns, persistence).await?;
            Ok(true)
        }
        (true, IfExists::Abort) => Err(AppError::Config(format!(
            "目标表 {} 已存在（默认策略 abort）",
            target.quoted()
        ))),
        (true, IfExists::Append) => {
            validate_append(transaction, target, columns).await?;
            Ok(false)
        }
        (true, IfExists::Truncate) => {
            transaction
                .execute(&format!("TRUNCATE TABLE {}", target.quoted()), &[])
                .await?;
            validate_append(transaction, target, columns).await?;
            Ok(false)
        }
        (true, IfExists::Replace) => {
            transaction
                .execute(&format!("DROP TABLE {}", target.quoted()), &[])
                .await?;
            create_table(transaction, target, columns, persistence).await?;
            Ok(true)
        }
    }
}
