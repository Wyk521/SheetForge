use crate::pg_import::config::ConnectionProfile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
