use crate::model::{
    build_output_plan, source_to_output_map, MergeMode, MergeOptions, OutputPlan, SourceKind,
    SourceTable,
};
use crate::scan::for_each_csv_row;
use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub const XLSX_MAX_ROWS: u32 = 1_048_576;
pub const XLSX_MAX_DATA_ROWS: u32 = XLSX_MAX_ROWS - 1;

#[derive(Debug)]
pub enum MergeEvent {
    Progress { current: u64, total: u64, label: String },
    Finished { output: PathBuf, rows: u64, sheets: usize },
    Cancelled,
    Failed(String),
}

pub fn spawn_merge(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    output: PathBuf,
    tx: Sender<MergeEvent>,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || match merge_tables(&tables, &options, &output, &tx, &cancel) {
        Ok(Some((rows, sheets))) => {
            let _ = tx.send(MergeEvent::Finished {
                output,
                rows,
                sheets,
            });
        }
        Ok(None) => {
            let _ = tx.send(MergeEvent::Cancelled);
        }
        Err(error) => {
            let _ = tx.send(MergeEvent::Failed(format!("{error:#}")));
        }
    });
}

fn merge_tables(
    tables: &[SourceTable],
    options: &MergeOptions,
    output: &Path,
    tx: &Sender<MergeEvent>,
    cancel: &AtomicBool,
) -> Result<Option<(u64, usize)>> {
    let enabled: Vec<&SourceTable> = tables.iter().filter(|table| table.enabled).collect();
    if enabled.is_empty() {
        return Err(anyhow!("没有勾选任何表"));
    }
    let plan = build_output_plan(tables, options);
    if plan.headers.is_empty() {
        return Err(anyhow!("合并后没有可输出的列，请检查合并模式或手动映射"));
    }
    if plan.headers.len() > 16_384 {
        return Err(anyhow!("输出列数超过 XLSX 的 16,384 列限制"));
    }

    let total: u64 = enabled.iter().map(|table| table.estimated_rows).sum();
    let mut sink = XlsxSink::new(plan.clone())?;
    let mut current = 0_u64;

    for table in enabled {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let label = table.display_name();
        let map = source_to_output_map(table, &plan, options.mode);
        let source_file = table
            .path
            .file_name()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut consume = |values: Vec<CellValue>| -> Result<()> {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("__CANCELLED__"));
            }
            let mut output_row = vec![CellValue::Empty; plan.headers.len()];
            for (source_index, output_index) in &map {
                if let Some(value) = values.get(*source_index) {
                    if output_row[*output_index].is_empty() && !value.is_empty() {
                        output_row[*output_index] = value.clone();
                    }
                }
            }
            if let Some(index) = plan.source_file_column {
                output_row[index] = CellValue::Text(source_file.clone());
            }
            if let Some(index) = plan.source_sheet_column {
                output_row[index] = CellValue::Text(table.sheet_name.clone());
            }
            sink.write_row(&output_row)?;
            current += 1;
            if current % 1_000 == 0 || current == total {
                let _ = tx.send(MergeEvent::Progress {
                    current,
                    total,
                    label: label.clone(),
                });
            }
            Ok(())
        };

        let result = match table.kind {
            SourceKind::Csv { delimiter } => for_each_csv_row(&table.path, delimiter, |row| {
                consume(row.into_iter().map(CellValue::Text).collect())
            }),
            SourceKind::Workbook => read_workbook_sheet(table, &mut consume),
        };
        if let Err(error) = result {
            if error.to_string().contains("__CANCELLED__") {
                return Ok(None);
            }
            return Err(error).with_context(|| format!("处理 {} 时失败", table.display_name()));
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let sheets = sink.sheet_count();
    sink.save(output)?;
    Ok(Some((current, sheets)))
}

fn read_workbook_sheet<F>(table: &SourceTable, callback: &mut F) -> Result<()>
where
    F: FnMut(Vec<CellValue>) -> Result<()>,
{
    let mut workbook = open_workbook_auto(&table.path)?;
    let range = workbook.worksheet_range(&table.sheet_name)?;
    for row in range.rows().skip(1) {
        callback(row.iter().map(CellValue::from_calamine).collect())?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum CellValue {
    Empty,
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
}

impl CellValue {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty) || matches!(self, Self::Text(value) if value.is_empty())
    }

    fn from_calamine(value: &Data) -> Self {
        match value {
            Data::Empty => Self::Empty,
            Data::String(value) => Self::Text(value.clone()),
            Data::Int(value) => Self::Integer(*value),
            Data::Float(value) => Self::Number(*value),
            Data::Bool(value) => Self::Boolean(*value),
            Data::DateTime(value) => Self::Text(value.to_string()),
            Data::DateTimeIso(value) | Data::DurationIso(value) => Self::Text(value.clone()),
            Data::Error(value) => Self::Text(value.to_string()),
        }
    }
}

struct XlsxSink {
    workbook: Workbook,
    plan: OutputPlan,
    header_format: Format,
    current_sheet: usize,
    row_in_sheet: u32,
    max_data_rows: u32,
    total_rows: u64,
}

impl XlsxSink {
    fn new(plan: OutputPlan) -> Result<Self> {
        Self::new_with_limit(plan, XLSX_MAX_DATA_ROWS)
    }

    fn new_with_limit(plan: OutputPlan, max_data_rows: u32) -> Result<Self> {
        if max_data_rows == 0 || max_data_rows > XLSX_MAX_DATA_ROWS {
            return Err(anyhow!("无效的 Sheet 行数限制"));
        }
        let header_format = Format::new()
            .set_bold()
            .set_font_color(Color::White)
            .set_background_color(Color::RGB(0x2563EB))
            .set_align(FormatAlign::Center)
            .set_border(FormatBorder::Thin);
        let mut sink = Self {
            workbook: Workbook::new(),
            plan,
            header_format,
            current_sheet: 0,
            row_in_sheet: 0,
            max_data_rows,
            total_rows: 0,
        };
        sink.add_sheet()?;
        Ok(sink)
    }

    fn add_sheet(&mut self) -> Result<()> {
        let index = self.workbook.worksheets().len();
        let name = format!("合并结果_{:03}", index + 1);
        let worksheet = self.workbook.add_worksheet_with_constant_memory();
        worksheet.set_name(&name)?;
        worksheet.set_freeze_panes(1, 0)?;
        for (column, header) in self.plan.headers.iter().enumerate() {
            worksheet.write_with_format(0, column as u16, header, &self.header_format)?;
            worksheet.set_column_width(column as u16, suggested_width(header))?;
        }
        self.current_sheet = index;
        self.row_in_sheet = 0;
        Ok(())
    }

    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        if self.row_in_sheet >= self.max_data_rows {
            self.add_sheet()?;
        }
        let excel_row = self.row_in_sheet + 1;
        let worksheet = self.workbook.worksheet_from_index(self.current_sheet)?;
        for (column, value) in values.iter().enumerate() {
            match value {
                CellValue::Empty => {}
                CellValue::Text(value) => {
                    worksheet.write_string(excel_row, column as u16, value)?;
                }
                CellValue::Integer(value) => {
                    worksheet.write_number(excel_row, column as u16, *value as f64)?;
                }
                CellValue::Number(value) => {
                    worksheet.write_number(excel_row, column as u16, *value)?;
                }
                CellValue::Boolean(value) => {
                    worksheet.write_boolean(excel_row, column as u16, *value)?;
                }
            }
        }
        self.row_in_sheet += 1;
        self.total_rows += 1;
        Ok(())
    }

    fn sheet_count(&self) -> usize {
        self.current_sheet + 1
    }

    fn save(mut self, output: &Path) -> Result<()> {
        self.workbook
            .save(output)
            .with_context(|| format!("无法写入 {}，请确认文件未被 Excel 占用", output.display()))
    }
}

fn suggested_width(header: &str) -> f64 {
    let display_units: usize = header
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum();
    (display_units as f64 + 4.0).clamp(12.0, 32.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_capacity_reserves_one_header_row() {
        assert_eq!(XLSX_MAX_DATA_ROWS, 1_048_575);
    }

    #[test]
    fn chinese_headers_get_wider_columns() {
        assert_eq!(suggested_width("订单编号"), 12.0);
        assert!(suggested_width("a very long english heading") > 20.0);
    }

    #[test]
    fn sink_splits_rows_and_repeats_headers() {
        let plan = OutputPlan {
            headers: vec!["姓名".to_owned()],
            source_file_column: None,
            source_sheet_column: None,
        };
        let mut sink = XlsxSink::new_with_limit(plan, 2).unwrap();
        sink.write_row(&[CellValue::Text("甲".to_owned())]).unwrap();
        sink.write_row(&[CellValue::Text("乙".to_owned())]).unwrap();
        sink.write_row(&[CellValue::Text("丙".to_owned())]).unwrap();
        assert_eq!(sink.sheet_count(), 2);

        let output = std::env::temp_dir().join(format!(
            "sheetforge-split-test-{}.xlsx",
            std::process::id()
        ));
        sink.save(&output).unwrap();
        let mut workbook = open_workbook_auto(&output).unwrap();
        assert_eq!(workbook.sheet_names().len(), 2);
        assert_eq!(workbook.worksheet_range("合并结果_001").unwrap().height(), 3);
        assert_eq!(workbook.worksheet_range("合并结果_002").unwrap().height(), 2);
        let _ = std::fs::remove_file(output);
    }
}
