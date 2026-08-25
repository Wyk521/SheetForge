use super::{AppError, Result};

const SERVICE: &str = "pg-table-importer";

fn entry(profile_name: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, profile_name)
        .map_err(|error| AppError::Config(format!("无法访问系统凭据管理器: {error}")))
}

pub fn store(profile_name: &str, password: &str) -> Result<()> {
    entry(profile_name)?
        .set_password(password)
        .map_err(|error| AppError::Config(format!("无法保存系统凭据: {error}")))
}

pub fn get(profile_name: &str) -> Option<String> {
    entry(profile_name)
        .and_then(|entry| {
            entry
                .get_password()
                .map_err(|error| AppError::Config(error.to_string()))
        })
        .ok()
        .or_else(|| std::env::var("PGPASSWORD").ok())
}

pub fn delete(profile_name: &str) -> Result<()> {
    let credential = entry(profile_name)?;
    match credential.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Config(format!("无法删除系统凭据: {error}"))),
    }
}
