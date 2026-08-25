use std::time::{Duration, Instant};

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use serde::{Deserialize, Serialize};
use tokio_postgres::{config::SslMode, Client, Config};

use crate::pg_import::{
    config::ConnectionProfile,
    error::{AppError, Result},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub server_version: String,
    pub database: String,
    pub user: String,
    pub elapsed_ms: u128,
}

fn ssl_mode(value: &str) -> Result<SslMode> {
    match value.to_ascii_lowercase().as_str() {
        "disable" => Ok(SslMode::Disable),
        "allow" | "prefer" => Ok(SslMode::Prefer),
        "require" | "verify-ca" | "verify-full" => Ok(SslMode::Require),
        other => Err(AppError::Config(format!(
            "不支持的 sslmode“{other}”；可选 disable、prefer、require、verify-ca、verify-full"
        ))),
    }
}

pub async fn connect(
    profile: &ConnectionProfile,
    password: Option<&str>,
) -> Result<(Client, ConnectionInfo)> {
    let started = Instant::now();
    let mut config = Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .dbname(&profile.database)
        .user(&profile.user)
        .ssl_mode(ssl_mode(&profile.sslmode)?)
        .connect_timeout(Duration::from_secs(15));
    if let Some(password) = password {
        config.password(password);
    }

    let tls = TlsConnector::builder()
        .build()
        .map(MakeTlsConnector::new)
        .map_err(|error| AppError::context("初始化 TLS", None, None, error))?;
    let (client, connection) = config
        .connect(tls)
        .await
        .map_err(|error| AppError::context("连接 PostgreSQL", None, None, error))?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::error!(error = %error, "PostgreSQL 连接已中断");
        }
    });

    let row = client
        .query_one("SELECT version(), current_database(), current_user", &[])
        .await?;
    let info = ConnectionInfo {
        server_version: row.get(0),
        database: row.get(1),
        user: row.get(2),
        elapsed_ms: started.elapsed().as_millis(),
    };
    Ok((client, info))
}
