use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use super::{AppError, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionProfile {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub sslmode: String,
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            user: "postgres".into(),
            sslmode: "prefer".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub profiles: BTreeMap<String, ConnectionProfile>,
}

#[must_use]
pub fn default_config_path() -> PathBuf {
    BaseDirs::new().map_or_else(
        || PathBuf::from("pg-table-importer-config.toml"),
        |dirs| {
            dirs.config_dir()
                .join("pg-table-importer")
                .join("config.toml")
        },
    )
}

#[must_use]
pub fn default_data_dir() -> PathBuf {
    BaseDirs::new().map_or_else(
        || PathBuf::from("."),
        |dirs| dirs.data_dir().join("pg-table-importer"),
    )
}

pub fn load(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let text = fs::read_to_string(path)?;
    toml::from_str(&text)
        .map_err(|error| AppError::Config(format!("无法解析 {}: {error}", path.display())))
}

pub fn save(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(config)
        .map_err(|error| AppError::Config(format!("无法序列化配置: {error}")))?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_is_not_part_of_profile_serialization() {
        let text = toml::to_string(&ConnectionProfile::default()).unwrap();
        assert!(!text.to_lowercase().contains("password"));
    }
}
