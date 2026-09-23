use std::time::{Duration, Instant};

use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use serde::{Deserialize, Serialize};
use tokio_postgres::{config::SslMode, Client, Config, NoTls};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileSslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

fn ssl_mode(value: &str) -> Result<ProfileSslMode> {
    match value.to_ascii_lowercase().as_str() {
        "disable" => Ok(ProfileSslMode::Disable),
        "allow" => Ok(ProfileSslMode::Allow),
        "prefer" => Ok(ProfileSslMode::Prefer),
        "require" => Ok(ProfileSslMode::Require),
        "verify-ca" => Ok(ProfileSslMode::VerifyCa),
        "verify-full" => Ok(ProfileSslMode::VerifyFull),
        other => Err(AppError::Config(format!(
            "不支持的 sslmode“{other}”；可选 disable、allow、prefer、require、verify-ca、verify-full"
        ))),
    }
}

fn tls_connector(mode: ProfileSslMode) -> Result<MakeTlsConnector> {
    let mut builder = TlsConnector::builder();
    match mode {
        ProfileSslMode::Allow | ProfileSslMode::Prefer | ProfileSslMode::Require => {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        ProfileSslMode::VerifyCa => {
            builder.danger_accept_invalid_hostnames(true);
        }
        ProfileSslMode::VerifyFull | ProfileSslMode::Disable => {}
    }
    builder
        .build()
        .map(MakeTlsConnector::new)
        .map_err(|error| AppError::context("初始化 TLS", None, None, error))
}

pub async fn connect(
    profile: &ConnectionProfile,
    password: Option<&str>,
) -> Result<(Client, ConnectionInfo)> {
    let started = Instant::now();
    let mode = ssl_mode(&profile.sslmode)?;
    let mut config = Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .dbname(&profile.database)
        .user(&profile.user)
        .connect_timeout(Duration::from_secs(15));
    if let Some(password) = password {
        config.password(password);
    }

    let client = if mode == ProfileSslMode::Disable || mode == ProfileSslMode::Allow {
        config.ssl_mode(SslMode::Disable);
        match config.connect(NoTls).await {
            Ok((client, connection)) => {
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        tracing::error!(error = %error, "PostgreSQL 连接已中断");
                    }
                });
                client
            }
            Err(error) if mode == ProfileSslMode::Allow => {
                config.ssl_mode(SslMode::Require);
                let (client, connection) =
                    config
                        .connect(tls_connector(mode)?)
                        .await
                        .map_err(|tls_error| {
                            AppError::Config(format!(
                                "非 TLS 连接失败：{error}；TLS 重试失败：{tls_error}"
                            ))
                        })?;
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        tracing::error!(error = %error, "PostgreSQL 连接已中断");
                    }
                });
                client
            }
            Err(error) => {
                return Err(AppError::context("连接 PostgreSQL", None, None, error));
            }
        }
    } else {
        config.ssl_mode(if mode == ProfileSslMode::Prefer {
            SslMode::Prefer
        } else {
            SslMode::Require
        });
        let (client, connection) = config
            .connect(tls_connector(mode)?)
            .await
            .map_err(|error| AppError::context("连接 PostgreSQL", None, None, error))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::error!(error = %error, "PostgreSQL 连接已中断");
            }
        });
        client
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssl_modes_remain_distinct() {
        assert_eq!(ssl_mode("allow").unwrap(), ProfileSslMode::Allow);
        assert_eq!(ssl_mode("prefer").unwrap(), ProfileSslMode::Prefer);
        assert_eq!(ssl_mode("require").unwrap(), ProfileSslMode::Require);
        assert_eq!(ssl_mode("verify-ca").unwrap(), ProfileSslMode::VerifyCa);
        assert_eq!(ssl_mode("verify-full").unwrap(), ProfileSslMode::VerifyFull);
    }
}
