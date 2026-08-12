use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    Union,
    Intersection,
    Manual,
}

impl MergeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Union => "列名并集",
            Self::Intersection => "列名交集",
            Self::Manual => "手动映射",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum SourceKind {
    Csv { delimiter: u8 },
    Workbook,
}

#[derive(Clone, Debug)]
pub struct ColumnMapping {
    pub source_index: usize,
    pub source_name: String,
    pub target_name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct SourceTable {
    pub path: PathBuf,
    pub sheet_name: String,
    pub kind: SourceKind,
    pub headers: Vec<String>,
    pub estimated_rows: u64,
    pub enabled: bool,
    pub mappings: Vec<ColumnMapping>,
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

#[derive(Clone, Debug)]
pub struct MergeOptions {
    pub mode: MergeMode,
    pub include_source_file: bool,
    pub include_source_sheet: bool,
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
        })
        .collect()
}

pub fn build_output_plan(tables: &[SourceTable], options: &MergeOptions) -> OutputPlan {
    let enabled: Vec<&SourceTable> = tables.iter().filter(|table| table.enabled).collect();
    let mut headers = match options.mode {
        MergeMode::Union => union_headers(&enabled),
        MergeMode::Intersection => intersection_headers(&enabled),
        MergeMode::Manual => manual_headers(&enabled),
    };

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
        MergeMode::Union | MergeMode::Intersection => table
            .headers
            .iter()
            .enumerate()
            .filter_map(|(source_index, name)| {
                output_indices
                    .get(&header_key(name))
                    .copied()
                    .map(|output_index| (source_index, output_index))
            })
            .collect(),
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
                include_source_file: false,
                include_source_sheet: false,
            },
        );
        let intersection = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Intersection,
                include_source_file: false,
                include_source_sheet: false,
            },
        );
        assert_eq!(union.headers, vec!["姓名", "年龄", "城市"]);
        assert_eq!(intersection.headers, vec!["姓名"]);
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
                include_source_file: false,
                include_source_sheet: false,
            },
        );
        assert_eq!(plan.headers, vec!["电话"]);
    }
}
