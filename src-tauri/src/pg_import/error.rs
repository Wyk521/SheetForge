use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("阶段“{stage}”失败{context}: {source}")]
    Context {
        stage: &'static str,
        context: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("路径不存在或不可访问: {0}")]
    InvalidPath(PathBuf),
    #[error("没有找到可用数据源")]
    NoSources,
    #[error("交集合并结果没有任何公共字段，已禁止导入")]
    EmptyIntersection,
    #[error("数据源 {source_name} 的表头重复: {headers:?}")]
    DuplicateHeaders {
        source_name: String,
        headers: Vec<String>,
    },
    #[error("数据源 {source_name} 第 {row} 行有 {actual} 个字段，表头有 {expected} 个字段")]
    RaggedRow {
        source_name: String,
        row: u64,
        expected: usize,
        actual: usize,
    },
    #[error("无法确认文本编码: {path}")]
    Encoding { path: PathBuf },
    #[error("不支持的编码“{0}”；可选 auto、utf-8、gbk、gb18030")]
    UnsupportedEncoding(String),
    #[error("不支持的分隔符“{0}”；请使用单个 ASCII 字符、tab 或 \\t")]
    InvalidDelimiter(String),
    #[error("配置错误: {0}")]
    Config(String),
    #[error("PostgreSQL 错误: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV/TXT 解析错误: {0}")]
    Csv(#[from] csv::Error),
    #[error("XLSX 写入错误: {0}")]
    Xlsx(#[from] rust_xlsxwriter::XlsxError),
    #[error("Excel 解析错误: {0}")]
    Excel(#[from] calamine::Error),
    #[error("序列化错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("操作已由用户取消，数据库事务已回滚")]
    Cancelled,
}

impl AppError {
    pub fn context(
        stage: &'static str,
        file: Option<&std::path::Path>,
        sheet: Option<&str>,
        source: impl Into<anyhow::Error>,
    ) -> Self {
        let mut parts = Vec::new();
        if let Some(file) = file {
            parts.push(format!("文件={}", file.display()));
        }
        if let Some(sheet) = sheet {
            parts.push(format!("工作表={sheet}"));
        }
        let context = if parts.is_empty() {
            String::new()
        } else {
            format!("（{}）", parts.join("，"))
        };
        Self::Context {
            stage,
            context,
            source: source.into(),
        }
    }
}
