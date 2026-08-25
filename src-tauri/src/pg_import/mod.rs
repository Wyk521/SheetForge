//! PostgreSQL 导入核心。
//!
//! 这些实现融合自 pg-table-importer 0.2.1（提交
//! 55c9f7df4897307121ac37f7066cc892d4c27ba4），现在属于 SheetForge
//! 自身源码，不依赖外部仓库即可构建。

// 保留上游导入核心的完整公共接口，部分接口暂未被 GUI 路径直接调用。
#![allow(dead_code)]

pub mod config;
pub mod credentials;
pub mod error;
pub mod postgres;
pub mod schema;
pub mod transform;

pub use error::{AppError, Result};
