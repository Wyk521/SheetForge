use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeMode {
    Manual,
    Intersection,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SourceKind {
    Csv { delimiter: u8 },
    Workbook,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformOp {
    #[default]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub source_index: usize,
    pub source_name: String,
    pub target_name: String,
    pub enabled: bool,
    #[serde(default)]
    pub transform: TransformOp,
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
    pub filter_column: String,
    #[serde(default)]
    pub filter_text: String,
    #[serde(default)]
    pub filter_exclude: bool,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            mode: MergeMode::Manual,
            include_source_file: false,
            include_source_sheet: false,
            output_order: Vec::new(),
            deduplicate: false,
            filter_column: String::new(),
            filter_text: String::new(),
            filter_exclude: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OutputPlan {
    pub headers: Vec<String>,
    pub source_file_column: Option<usize>,
    pub source_sheet_column: Option<usize>,
}

pub fn header_key(value: &str) -> String {
    value
        .split(['\r', '\n', '\u{2028}', '\u{2029}'])
        .map(str::trim)
        .collect::<String>()
        .to_lowercase()
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
        })
        .collect()
}

pub fn build_output_plan(tables: &[SourceTable], options: &MergeOptions) -> OutputPlan {
    let enabled: Vec<&SourceTable> = tables.iter().filter(|table| table.enabled).collect();
    let mut headers = match options.mode {
        MergeMode::Manual => manual_headers(&enabled),
        MergeMode::Intersection => intersection_headers(&enabled),
    };

    // 来源列先并入列集，使其能参与 output_order 排序（可排在任意位置），
    // 而不是固定追加在末尾。
    let source_file_name = options
        .include_source_file
        .then(|| push_unique_metadata(&mut headers, "来源文件"));
    let source_sheet_name = options
        .include_source_sheet
        .then(|| push_unique_metadata(&mut headers, "来源工作表"));

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

    // 排序后重新定位来源列的实际位置
    let source_file_column = source_file_name.map(|name| {
        headers
            .iter()
            .position(|header| header_key(header) == header_key(&name))
            .expect("source file column was just inserted")
    });
    let source_sheet_column = source_sheet_name.map(|name| {
        headers
            .iter()
            .position(|header| header_key(header) == header_key(&name))
            .expect("source sheet column was just inserted")
    });

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

fn push_unique_metadata(headers: &mut Vec<String>, base: &str) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    let existing: HashSet<String> = headers.iter().map(|h| header_key(h)).collect();
    while existing.contains(&header_key(&name)) {
        name = format!("_{base}{suffix}");
        suffix += 1;
    }
    headers.push(name.clone());
    name
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
        MergeMode::Intersection => table
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
    fn line_breaks_in_headers_are_ignored_for_matching() {
        assert_eq!(header_key("客户\n姓名"), "客户姓名");
        assert_eq!(header_key("客户\r\n姓名"), "客户姓名");
        assert_eq!(header_key("客户 \n 姓名"), "客户姓名");
    }

    #[test]
    fn manual_mapping_with_line_breaks_does_not_create_duplicate_output_column() {
        let mut wrapped = table(&["客户\n姓名"]);
        let plain = table(&["客户姓名"]);
        wrapped.mappings[0].target_name = "客户姓名".to_owned();
        let plan = build_output_plan(
            &[wrapped, plain],
            &MergeOptions {
                mode: MergeMode::Manual,
                ..Default::default()
            },
        );
        assert_eq!(plan.headers, vec!["客户姓名"]);
    }

    #[test]
    fn manual_and_intersection_preserve_first_table_order() {
        let tables = vec![table(&["姓名", "年龄"]), table(&["姓名", "城市"])];
        let manual = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Manual,
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
        assert_eq!(manual.headers, vec!["姓名", "年龄", "城市"]);
        assert_eq!(intersection.headers, vec!["姓名"]);
    }

    #[test]
    fn manual_includes_enabled_mapping_targets_and_common_headers_are_detected() {
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
    fn source_sheet_column_follows_user_output_order() {
        // 用户场景：开启「记录来源工作表」后，把「来源工作表」列拖到输出顺序最前面
        let tables = vec![table(&["姓名", "年龄"]), table(&["姓名", "城市"])];
        let plan = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Manual,
                include_source_sheet: true,
                output_order: vec![
                    "来源工作表".to_owned(),
                    "姓名".to_owned(),
                    "年龄".to_owned(),
                    "城市".to_owned(),
                ],
                ..Default::default()
            },
        );
        assert_eq!(plan.headers, vec!["来源工作表", "姓名", "年龄", "城市"]);
        assert_eq!(plan.source_sheet_column, Some(0));
    }

    #[test]
    fn source_columns_can_sit_in_the_middle_of_output() {
        // 用户场景：把「来源工作表」拖到「姓名」和「年龄」之间
        let tables = vec![table(&["姓名", "年龄"])];
        let plan = build_output_plan(
            &tables,
            &MergeOptions {
                mode: MergeMode::Manual,
                include_source_sheet: true,
                output_order: vec![
                    "姓名".to_owned(),
                    "来源工作表".to_owned(),
                    "年龄".to_owned(),
                ],
                ..Default::default()
            },
        );
        assert_eq!(plan.headers, vec!["姓名", "来源工作表", "年龄"]);
        assert_eq!(plan.source_sheet_column, Some(1));
    }

    #[test]
    fn merge_options_round_trip_as_scheme_data() {
        let options = MergeOptions {
            mode: MergeMode::Manual,
            deduplicate: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&options).unwrap();
        let restored: MergeOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, MergeMode::Manual);
        assert!(restored.deduplicate);
    }

    #[test]
    fn source_metadata_keeps_original_column_with_same_name() {
        let plan = build_output_plan(
            &[table(&["来源文件", "来源工作表", "值"])],
            &MergeOptions {
                include_source_file: true,
                include_source_sheet: true,
                ..Default::default()
            },
        );
        assert_eq!(
            plan.headers,
            vec!["来源文件", "来源工作表", "值", "_来源文件2", "_来源工作表2"]
        );
        assert_eq!(plan.source_file_column, Some(3));
        assert_eq!(plan.source_sheet_column, Some(4));
    }
}
