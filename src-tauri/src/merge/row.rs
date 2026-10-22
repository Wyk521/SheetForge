use crate::model::{
    header_key, source_to_output_map, MergeMode, MergeOptions, OutputPlan, SourceKind, SourceTable,
    TransformOp,
};
use calamine::Data;
use std::{borrow::Cow, collections::HashMap};
pub(super) fn is_blank_source_row(kind: SourceKind, values: &[CellValue]) -> bool {
    match kind {
        SourceKind::Csv { .. } => values.iter().all(CellValue::is_empty),
        SourceKind::Workbook => values.iter().all(|value| matches!(value, CellValue::Empty)),
    }
}

pub(super) struct RowMapping {
    /// (源列、输出列、转换操作、是否可以从输入行直接移动值)。
    columns: Vec<(usize, usize, TransformOp, bool)>,
    source_file: Option<String>,
    source_sheet: Option<String>,
    pub(super) label: String,
}

pub(super) fn build_row_mapping(
    table: &SourceTable,
    plan: &OutputPlan,
    mode: MergeMode,
) -> RowMapping {
    let columns = source_to_output_map(table, plan, mode);
    let mut source_use_count = HashMap::<usize, usize>::new();
    for (source_index, _) in &columns {
        *source_use_count.entry(*source_index).or_default() += 1;
    }

    // 与 mapped_row 之前的 find 语义保持一致：重复 source_index 时取第一条规则。
    // 列名交集直接使用原始表头和原值，不应用「修正表头」中设置的清洗。
    let mut transforms = vec![None; table.headers.len()];
    let mappings = match mode {
        MergeMode::Manual => table.mappings.as_slice(),
        MergeMode::Intersection => &[],
    };
    for mapping in mappings {
        if let Some(transform) = transforms.get_mut(mapping.source_index) {
            if transform.is_none() {
                *transform = Some(mapping.transform);
            }
        }
    }

    let columns = columns
        .into_iter()
        .map(|(source_index, output_index)| {
            let transform = transforms
                .get(source_index)
                .and_then(|value| *value)
                .unwrap_or(TransformOp::None);
            let can_move = transform == TransformOp::None
                && source_use_count.get(&source_index).copied() == Some(1);
            (source_index, output_index, transform, can_move)
        })
        .collect();

    RowMapping {
        label: table.display_name(),
        columns,
        source_file: plan.source_file_column.map(|_| {
            table
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        source_sheet: plan.source_sheet_column.map(|_| table.sheet_name.clone()),
    }
}

pub(super) fn mapped_row(
    values: Vec<CellValue>,
    row_mapping: &RowMapping,
    plan: &OutputPlan,
) -> Vec<CellValue> {
    let mut values = values;
    let mut output = vec![CellValue::Empty; plan.headers.len()];
    for &(source_index, output_index, transform, can_move) in &row_mapping.columns {
        if source_index < values.len() {
            let value = if can_move {
                std::mem::replace(&mut values[source_index], CellValue::Empty)
            } else {
                values[source_index].transformed(transform)
            };
            if output[output_index].is_empty() && !value.is_empty() {
                output[output_index] = value;
            }
        }
    }
    if let Some(index) = plan.source_file_column {
        output[index] = CellValue::Text(row_mapping.source_file.clone().unwrap_or_default());
    }
    if let Some(index) = plan.source_sheet_column {
        output[index] = CellValue::Text(row_mapping.source_sheet.clone().unwrap_or_default());
    }
    output
}

pub(crate) fn preview_mapped_row(
    source: Vec<CellValue>,
    table: &SourceTable,
    plan: &OutputPlan,
    options: &MergeOptions,
) -> Option<Vec<String>> {
    if is_blank_source_row(table.kind, &source) {
        return None;
    }
    let mapping = build_row_mapping(table, plan, options.mode);
    let output = mapped_row(source, &mapping, plan);
    RowFilter::new(plan, options)
        .accepts(&output)
        .then(|| row_key(&output))
}

pub(super) fn row_key(row: &[CellValue]) -> Vec<String> {
    row.iter().map(CellValue::as_text).collect()
}

pub(super) struct RowFilter {
    column: Option<usize>,
    needle: String,
    exclude: bool,
}
impl RowFilter {
    pub(super) fn new(plan: &OutputPlan, options: &MergeOptions) -> Self {
        let key = header_key(&options.filter_column);
        let column = (!options.filter_text.is_empty())
            .then(|| {
                plan.headers
                    .iter()
                    .position(|header| header_key(header) == key)
            })
            .flatten();
        Self {
            column,
            needle: options.filter_text.to_lowercase(),
            exclude: options.filter_exclude,
        }
    }
    pub(super) fn accepts(&self, row: &[CellValue]) -> bool {
        let Some(column) = self.column else {
            return true;
        };
        let value = row
            .get(column)
            .and_then(|value| value.copy_text(false))
            .unwrap_or_default();
        let contains = value.to_lowercase().contains(&self.needle);
        if self.exclude {
            !contains
        } else {
            contains
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum CellValue {
    Empty,
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    /// Excel 日期/时间，由源文件解析出的年月日时分秒组成。
    DateTime {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        millisecond: u16,
    },
    /// 仅含时间的单元格（Excel 序列值小于 1），如 08:30。
    Time {
        hour: u8,
        minute: u8,
        second: u8,
        millisecond: u16,
    },
}

impl CellValue {
    pub(super) fn is_empty(&self) -> bool {
        matches!(self, Self::Empty) || matches!(self, Self::Text(value) if value.is_empty())
    }
    pub(crate) fn copy_text(&self, empty_as_null: bool) -> Option<Cow<'_, str>> {
        match self {
            Self::Empty => None,
            Self::Text(v) if empty_as_null && v.is_empty() => None,
            Self::Text(v) => Some(Cow::Borrowed(v)),
            Self::Integer(v) => Some(Cow::Owned(v.to_string())),
            Self::Number(v) => Some(Cow::Owned(v.to_string())),
            Self::Boolean(v) => Some(Cow::Owned(v.to_string())),
            Self::DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
                millisecond,
            } if *hour == 0 && *minute == 0 && *second == 0 && *millisecond == 0 => {
                Some(Cow::Owned(format!("{year:04}-{month:02}-{day:02}")))
            }
            Self::DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
                millisecond,
            } => {
                let mut text =
                    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}");
                if *millisecond > 0 {
                    text.push_str(&format!(".{millisecond:03}"));
                }
                Some(Cow::Owned(text))
            }
            Self::Time {
                hour,
                minute,
                second,
                millisecond,
            } => {
                let mut text = format!("{hour:02}:{minute:02}:{second:02}");
                if *millisecond > 0 {
                    text.push_str(&format!(".{millisecond:03}"));
                }
                Some(Cow::Owned(text))
            }
        }
    }
    pub(super) fn as_text(&self) -> String {
        self.copy_text(false)
            .map(Cow::into_owned)
            .unwrap_or_default()
    }
    pub(super) fn transformed(&self, operation: TransformOp) -> Self {
        // 去空格和大小写只对文本有意义；数字、布尔、日期保持原类型，
        // 否则 XLSX 中会变成“以文本形式存储的数字”。
        match self {
            Self::Text(value) if operation != TransformOp::None => {
                Self::Text(operation.apply(value))
            }
            other => other.clone(),
        }
    }
    pub(crate) fn from_calamine_owned(value: Data) -> Self {
        match value {
            Data::String(value) => Self::Text(value),
            Data::DateTimeIso(value) | Data::DurationIso(value) => Self::Text(value),
            other => Self::from_calamine(&other),
        }
    }
    pub(crate) fn from_calamine(value: &Data) -> Self {
        match value {
            Data::Empty => Self::Empty,
            Data::String(v) => Self::Text(v.clone()),
            Data::Int(v) => Self::Integer(*v),
            Data::Float(v) => Self::Number(*v),
            Data::Bool(v) => Self::Boolean(*v),
            Data::DateTime(v) => {
                // 时长格式（如 [hh]:mm:ss）保持数值文本。
                if v.is_duration() {
                    Self::Text(v.as_f64().to_string())
                } else {
                    let (year, month, day, hour, minute, second, millisecond) =
                        v.to_ymd_hms_milli();
                    if (0.0..1.0).contains(&v.as_f64()) {
                        return Self::Time {
                            hour,
                            minute,
                            second,
                            millisecond,
                        };
                    }
                    Self::DateTime {
                        year,
                        month,
                        day,
                        hour,
                        minute,
                        second,
                        millisecond,
                    }
                }
            }
            Data::DateTimeIso(v) | Data::DurationIso(v) => Self::Text(v.clone()),
            Data::Error(v) => Self::Text(v.to_string()),
        }
    }

    pub(crate) fn from_text(value: String) -> Self {
        if value.trim().is_empty() {
            Self::Empty
        } else {
            Self::Text(value)
        }
    }
}
