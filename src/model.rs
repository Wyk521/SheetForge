use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeMode {
    Union,
    Intersection,
    Manual,
    Consolidate,
    Join,
}

impl MergeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Union => "列名并集",
            Self::Intersection => "列名交集",
            Self::Manual => "手动映射",
            Self::Consolidate => "按键汇总",
            Self::Join => "横向关联",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SourceKind {
    Csv { delimiter: u8 },
    Workbook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformOp {
    None,
    Trim,
    Uppercase,
    Lowercase,
}

impl TransformOp {
    pub fn apply(self, value: &str) -> String {
        match self {
            Self::None => value.to_owned(),
            Self::Trim => value.trim().to_owned(),
            Self::Uppercase => value.trim().to_uppercase(),
            Self::Lowercase => value.trim().to_lowercase(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateOp {
    First,
    Sum,
    UniqueJoin,
    TextJoin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinKind {
    Left,
    Inner,
    Full,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub source_index: usize,
    pub source_name: String,
    pub target_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub transform: TransformOp,
    #[serde(default)]
    pub aggregate: AggregateOp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceTable {
    pub path: PathBuf,
    pub sheet_name: String,
    pub kind: SourceKind,
    pub header_row: usize,
    #[serde(default = "default_header_rows")]
    pub header_rows: usize,
    #[serde(default)]
    pub suggested_header_row: usize,
    pub headers: Vec<String>,
    pub estimated_rows: u64,
    pub enabled: bool,
    pub mappings: Vec<ColumnMapping>,
}

fn default_header_rows() -> usize {
    1
}

impl SourceTable {
    pub fn display_name(&self) -> String {
        let file = self
            .path
            .file_name()
            .map(|v| v.to_string_lossy())
            .unwrap_or_default();
        format!("{file}  /  {}", self.sheet_name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeOptions {
    pub mode: MergeMode,
    pub include_source_file: bool,
    pub include_source_sheet: bool,
    #[serde(default)]
    pub output_order: Vec<String>,
    #[serde(default)]
    pub deduplicate: bool,
    #[serde(default)]
    pub key_columns: Vec<String>,
    #[serde(default)]
    pub join_kind: JoinKind,
    #[serde(default = "default_join_separator")]
    pub text_join_separator: String,
    #[serde(default)]
    pub filter_column: String,
    #[serde(default)]
    pub filter_text: String,
    #[serde(default)]
    pub filter_exclude: bool,
}

fn default_join_separator() -> String {
    "；".to_owned()
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            mode: MergeMode::Union,
            include_source_file: false,
            include_source_sheet: false,
            output_order: Vec::new(),
            deduplicate: false,
            key_columns: Vec::new(),
            join_kind: JoinKind::Left,
            text_join_separator: default_join_separator(),
            filter_column: String::new(),
            filter_text: String::new(),
            filter_exclude: false,
        }
    }
}

impl Default for TransformOp {
    fn default() -> Self {
        Self::None
    }
}

impl Default for AggregateOp {
    fn default() -> Self {
        Self::First
    }
}

impl Default for JoinKind {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Clone, Debug)]
pub struct OutputPlan {
    pub headers: Vec<String>,
    pub source_file_column: Option<usize>,
    pub source_sheet_column: Option<usize>,
}

pub fn header_key(value: &str) -> String {
    value.trim().to_lowercase()
}

pub fn normalize_headers<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut used = HashSet::new();
    let mut result = Vec::new();

    for (index, value) in values.into_iter().enumerate() {
        let trimmed = value.as_ref().trim().trim_start_matches('\u{feff}');
        let base = if trimmed.is_empty() {
            format!("未命名列{}", index + 1)
        } else {
            trimmed.to_owned()
        };
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !used.insert(header_key(&candidate)) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        result.push(candidate);
    }
    result
}

pub fn make_default_mappings(headers: &[String]) -> Vec<ColumnMapping> {
    headers
        .iter()
        .enumerate()
        .map(|(source_index, name)| ColumnMapping {
            source_index,
            source_name: name.clone(),
            target_name: name.clone(),
            enabled: true,
            transform: TransformOp::None,
            aggregate: AggregateOp::First,
        })
        .collect()
}

pub fn build_output_plan(tables: &[SourceTable], options: &MergeOptions) -> OutputPlan {
    let enabled: Vec<&SourceTable> = tables.iter().filter(|table| table.enabled).collect();
    let mut headers = match options.mode {
        MergeMode::Union | MergeMode::Consolidate | MergeMode::Join => union_headers(&enabled),
        MergeMode::Intersection => intersection_headers(&enabled),
        MergeMode::Manual => manual_headers(&enabled),
    };

    if !options.output_order.is_empty() {
        let positions: HashMap<String, usize> = options
            .output_order
            .iter()
            .enumerate()
            .map(|(index, name)| (header_key(name), index))
            .collect();
        headers.sort_by_key(|name| {
            positions
                .get(&header_key(name))
                .copied()
                .unwrap_or(usize::MAX)
        });
    }

    let source_file_column = options
        .include_source_file
        .then(|| push_unique_metadata(&mut headers, "来源文件"));
    let source_sheet_column = options
        .include_source_sheet
        .then(|| push_unique_metadata(&mut headers, "来源工作表"));

    OutputPlan {
        headers,
        source_file_column,
        source_sheet_column,
    }
}

pub fn common_header_keys(tables: &[SourceTable]) -> HashSet<String> {
    let enabled: Vec<&SourceTable> = tables.iter().filter(|table| table.enabled).collect();
    if enabled.len() < 2 {
        return HashSet::new();
    }
    let first = enabled[0];
    let mut common: HashSet<String> = first.headers.iter().map(|h| header_key(h)).collect();
    for table in enabled.iter().skip(1) {
        let current: HashSet<String> = table.headers.iter().map(|h| header_key(h)).collect();
        common.retain(|key| current.contains(key));
    }
    common
}

fn union_headers(tables: &[&SourceTable]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for table in tables {
        for header in &table.headers {
            if seen.insert(header_key(header)) {
                output.push(header.clone());
            }
        }
    }
    output
}

fn intersection_headers(tables: &[&SourceTable]) -> Vec<String> {
    let Some(first) = tables.first() else {
        return Vec::new();
    };
    let other_sets: Vec<HashSet<String>> = tables
        .iter()
        .skip(1)
        .map(|table| table.headers.iter().map(|h| header_key(h)).collect())
        .collect();

    first
        .headers
        .iter()
        .filter(|header| {
            let key = header_key(header);
            other_sets.iter().all(|set| set.contains(&key))
        })
        .cloned()
        .collect()
}

fn manual_headers(tables: &[&SourceTable]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for table in tables {
        for mapping in table.mappings.iter().filter(|mapping| mapping.enabled) {
            let target = mapping.target_name.trim();
            if !target.is_empty() && seen.insert(header_key(target)) {
                output.push(target.to_owned());
            }
        }
    }
    output
}

fn push_unique_metadata(headers: &mut Vec<String>, base: &str) -> usize {
    let mut name = base.to_owned();
    let mut suffix = 2;
    let existing: HashSet<String> = headers.iter().map(|h| header_key(h)).collect();
    while existing.contains(&header_key(&name)) {
        name = format!("_{base}{suffix}");
        suffix += 1;
    }
    headers.push(name);
    headers.len() - 1
}

pub fn source_to_output_map(
    table: &SourceTable,
    plan: &OutputPlan,
    mode: MergeMode,
) -> Vec<(usize, usize)> {
    let output_indices: HashMap<String, usize> = plan
        .headers
        .iter()
        .enumerate()
        .map(|(index, name)| (header_key(name), index))
        .collect();

    match mode {
        MergeMode::Union | MergeMode::Intersection | MergeMode::Consolidate | MergeMode::Join => {
            table
                .headers
                .iter()
                .enumerate()
                .filter_map(|(source_index, name)| {
                    output_indices
                        .get(&header_key(name))
                        .copied()
                        .map(|output_index| (source_index, output_index))
                })
                .collect()
        }
        MergeMode::Manual => table
            .mappings
            .iter()
            .filter(|mapping| mapping.enabled && !mapping.target_name.trim().is_empty())
            .filter_map(|mapping| {
                output_indices
                    .get(&header_key(&mapping.target_name))
                    .copied()
                    .map(|output_index| (mapping.source_index, output_index))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(headers: &[&str]) -> SourceTable {
        let headers = headers.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        SourceTable {
            path: PathBuf::from("a.csv"),
            sheet_name: "CSV".to_owned(),
            kind: SourceKind::Csv { delimiter: b',' },
            header_row: 1,
            header_rows: 1,
            suggested_header_row: 1,
            mappings: make_default_mappings(&headers),
            headers,
            estimated_rows: 0,
            enabled: true,
        }
    }

    #[test]
    fn headers_are_unique_and_named() {
        assert_eq!(
            normalize_headers(["姓名", "", "姓名"]),
            vec!["姓名", "未命名列2", "姓名_2"]
        );
    }

    #[test]
    fn union_and_intersection_preserve_first_table_order() {
        let tables = vec![table(&["姓名", "年龄"]), table(&["姓名", "城市"])];
        let union = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Union,
                ..Default::default()
            },
        );
        let intersection = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Intersection,
                ..Default::default()
            },
        );
        assert_eq!(union.headers, vec!["姓名", "年龄", "城市"]);
        assert_eq!(intersection.headers, vec!["姓名"]);
    }

    #[test]
    fn manual_defaults_to_union_and_common_headers_are_detected() {
        let tables = vec![table(&["姓名", "手机号"]), table(&["姓名", "联系电话"])];
        let manual = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Manual,
                ..Default::default()
            },
        );
        assert_eq!(manual.headers, vec!["姓名", "手机号", "联系电话"]);
        assert_eq!(
            common_header_keys(&tables),
            HashSet::from(["姓名".to_owned()])
        );
    }

    #[test]
    fn manual_mapping_joins_different_source_names() {
        let mut a = table(&["手机号"]);
        let mut b = table(&["联系电话"]);
        a.mappings[0].target_name = "电话".to_owned();
        b.mappings[0].target_name = "电话".to_owned();
        let plan = build_output_plan(
            &[a, b],
            &MergeOptions {
                mode: MergeMode::Manual,
                ..Default::default()
            },
        );
        assert_eq!(plan.headers, vec!["电话"]);
    }

    #[test]
    fn merge_options_round_trip_as_scheme_data() {
        let options = MergeOptions {
            mode: MergeMode::Join,
            key_columns: vec!["订单号".to_owned(), "日期".to_owned()],
            join_kind: JoinKind::Full,
            deduplicate: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&options).unwrap();
        let restored: MergeOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, MergeMode::Join);
        assert_eq!(restored.join_kind, JoinKind::Full);
        assert_eq!(restored.key_columns.len(), 2);
    }
}
