use super::{CellValue, MergeSummary, RowSink};
use crate::model::OutputPlan;
use anyhow::{anyhow, Context, Result};
use rust_xlsxwriter::{Color, ExcelDateTime, Format, FormatAlign, FormatBorder, Workbook};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
pub const XLSX_MAX_ROWS: u32 = 1_048_576;
pub const XLSX_MAX_DATA_ROWS: u32 = XLSX_MAX_ROWS - 1;
pub(super) fn finish_sink(
    sink: XlsxSink,
    output: &Path,
    rows: u64,
    cancel: &AtomicBool,
) -> Result<Option<MergeSummary>> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let summary = MergeSummary {
        rows,
        sheets: sink.sheet_count(),
        truncated_cells: sink.truncated_cells,
    };
    sink.save(output)?;
    Ok(Some(summary))
}
pub(super) struct XlsxSink {
    workbook: Workbook,
    plan: OutputPlan,
    header_format: Format,
    current_sheet: usize,
    row_in_sheet: u32,
    max_data_rows: u32,
    pub(super) truncated_cells: u64,
}

/// Excel 单元格最多容纳 32,767 个字符。
const XLSX_MAX_CELL_CHARS: usize = 32_767;

impl XlsxSink {
    pub(super) fn new(plan: OutputPlan) -> Result<Self> {
        Self::new_with_limit(plan, XLSX_MAX_DATA_ROWS)
    }
    pub(super) fn new_with_limit(plan: OutputPlan, max_data_rows: u32) -> Result<Self> {
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
            truncated_cells: 0,
        };
        sink.add_sheet()?;
        Ok(sink)
    }
    fn add_sheet(&mut self) -> Result<()> {
        let index = self.workbook.worksheets().len();
        let worksheet = self.workbook.add_worksheet_with_constant_memory();
        worksheet.set_name(format!("合并结果_{:03}", index + 1))?;
        worksheet.set_freeze_panes(1, 0)?;
        for (column, header) in self.plan.headers.iter().enumerate() {
            worksheet.write_with_format(0, column as u16, header, &self.header_format)?;
            worksheet.set_column_width(column as u16, suggested_width(header))?;
        }
        self.current_sheet = index;
        self.row_in_sheet = 0;
        Ok(())
    }
    pub(super) fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        if self.row_in_sheet >= self.max_data_rows {
            self.add_sheet()?;
        }
        let excel_row = self.row_in_sheet + 1;
        let worksheet = self.workbook.worksheet_from_index(self.current_sheet)?;
        for (column, value) in values.iter().enumerate() {
            match value {
                CellValue::Empty => {}
                CellValue::Text(v) => {
                    // 超长文本按 Excel 上限截断并计数，而不是让整次合并在末尾失败。
                    match v.char_indices().nth(XLSX_MAX_CELL_CHARS) {
                        Some((end, _)) => {
                            worksheet.write_string(excel_row, column as u16, &v[..end])?;
                            self.truncated_cells += 1;
                        }
                        None => {
                            worksheet.write_string(excel_row, column as u16, v)?;
                        }
                    }
                }
                CellValue::Integer(v) => {
                    // f64 只能精确表示 2^53 以内的整数；超出的整数值（如 18 位
                    // 编号）以文本写出，避免尾数被静默截断。
                    if (*v as f64) as i64 == *v {
                        worksheet.write_number(excel_row, column as u16, *v as f64)?;
                    } else {
                        worksheet.write_string(excel_row, column as u16, v.to_string())?;
                    }
                }
                CellValue::Number(v) => {
                    worksheet.write_number(excel_row, column as u16, *v)?;
                }
                CellValue::Boolean(v) => {
                    worksheet.write_boolean(excel_row, column as u16, *v)?;
                }
                CellValue::DateTime {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    second,
                } => {
                    let datetime = ExcelDateTime::from_ymd(*year, *month, *day)?.and_hms(
                        *hour as u16,
                        *minute,
                        *second,
                    )?;
                    let format = if *hour == 0 && *minute == 0 && *second == 0 {
                        Format::new().set_num_format("yyyy-mm-dd")
                    } else {
                        Format::new().set_num_format("yyyy-mm-dd hh:mm:ss")
                    };
                    worksheet.write_datetime_with_format(
                        excel_row,
                        column as u16,
                        &datetime,
                        &format,
                    )?;
                }
                CellValue::Time {
                    hour,
                    minute,
                    second,
                } => {
                    let time = ExcelDateTime::from_hms(*hour as u16, *minute, *second)?;
                    worksheet.write_datetime_with_format(
                        excel_row,
                        column as u16,
                        &time,
                        &Format::new().set_num_format("hh:mm:ss"),
                    )?;
                }
            }
        }
        self.row_in_sheet += 1;
        Ok(())
    }
    pub(super) fn sheet_count(&self) -> usize {
        self.current_sheet + 1
    }
    pub(super) fn save(mut self, output: &Path) -> Result<()> {
        self.workbook
            .save(output)
            .with_context(|| format!("无法写入 {}，请确认文件未被 Excel 占用", output.display()))
    }
}

impl RowSink for XlsxSink {
    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        XlsxSink::write_row(self, values)
    }
}

pub(super) fn suggested_width(header: &str) -> f64 {
    let units: usize = header
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum();
    (units as f64 + 4.0).clamp(12.0, 32.0)
}
