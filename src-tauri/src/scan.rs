//! Compatibility entry and source reload coordination. Parsers are independent of Tauri.
mod csv;
mod discovery;
mod format;
mod header;
mod markup;
mod tasks;
mod workbook;

use self::csv::{csv_encoding_suspicious, scan_csv, scan_csv_auto};
// Keep the original module surface for callers while implementations live in domains.
#[allow(unused_imports)]
pub use self::csv::{
    decode_csv_field, decode_csv_field_with_quality, detect_delimiter, for_each_csv_row,
};
use crate::model::{header_key, SourceKind, SourceTable};
use anyhow::Result;
pub use discovery::{collect_folder, supported_file};
pub(crate) use format::{is_markup_workbook, is_xlsx_path};
#[allow(unused_imports)]
pub use header::recommend_header;
pub(crate) use markup::read_markup_workbook;
use std::{collections::HashSet, path::Path};
pub use tasks::{spawn_group_reload, spawn_scan, spawn_table_reload};
pub(crate) use workbook::{for_each_xlsx_row, open_detected_workbook, open_xlsx_workbook};
use workbook::{scan_workbook_auto_headers, scan_workbook_sheet, scan_xlsx_auto_headers};

pub fn scan_file(path: &Path) -> Result<(Vec<SourceTable>, Vec<String>)> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv" | "tsv") => {
            let table = scan_csv_auto(path)?;
            let delimiter = match table.kind {
                SourceKind::Csv { delimiter } => delimiter,
                SourceKind::Workbook => return Ok((vec![table], Vec::new())),
            };
            let mut warnings = Vec::new();
            if csv_encoding_suspicious(path, delimiter) {
                warnings.push(format!(
                    "{}：文件编码可能不是 UTF-8 或 GBK，部分字符无法识别",
                    path.display()
                ));
            }
            Ok((vec![table], warnings))
        }
        Some("xlsx" | "xlsm" | "xls" | "xlsb" | "ods") => {
            let tables = if is_xlsx_path(path) {
                scan_xlsx_auto_headers(path)?
            } else {
                scan_workbook_auto_headers(path)?
            };
            Ok((tables, Vec::new()))
        }
        _ => Ok((Vec::new(), Vec::new())),
    }
}

/// 抽样检查 CSV 前若干条记录的解码质量，出现替换字符（U+FFFD）视为可疑编码。
fn preserve_mappings(old: &SourceTable, new: &mut SourceTable) {
    for mapping in &mut new.mappings {
        if let Some(previous) = old
            .mappings
            .iter()
            .find(|previous| header_key(&previous.source_name) == header_key(&mapping.source_name))
        {
            let source_name = mapping.source_name.clone();
            let source_index = mapping.source_index;
            *mapping = previous.clone();
            mapping.source_name = source_name;
            mapping.source_index = source_index;
        }
    }
}

pub fn refresh_saved_table(old: &SourceTable) -> Result<SourceTable> {
    let mut current = match old.kind {
        SourceKind::Csv { delimiter } => scan_csv(
            &old.path,
            delimiter,
            old.header_row,
            old.header_rows,
            old.suggested_header_row,
        )?,
        SourceKind::Workbook => scan_workbook_sheet(
            &old.path,
            &old.sheet_name,
            old.header_row,
            old.header_rows,
            old.suggested_header_row,
        )?,
    };
    let saved = old
        .headers
        .iter()
        .map(|h| header_key(h))
        .collect::<HashSet<_>>();
    let found = current
        .headers
        .iter()
        .map(|h| header_key(h))
        .collect::<HashSet<_>>();
    if old.headers.len() != current.headers.len() || saved != found {
        anyhow::bail!("源表表头已变化，请重新扫描并配置字段映射");
    }
    current.enabled = old.enabled;
    preserve_mappings(old, &mut current);
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::header::*;
    use super::*;
    use calamine::{open_workbook, Data, Xlsx};
    use std::io::Write;
    use std::{fs::File, io::BufReader};

    #[test]
    fn header_recommendation_skips_report_title() {
        let rows = vec![
            vec!["2026 年销售报表".into()],
            vec!["姓名".into(), "金额".into(), "城市".into()],
            vec!["张三".into(), "12".into(), "北京".into()],
        ];
        assert_eq!(recommend_header(rows), 1);
    }

    #[test]
    fn multi_row_headers_are_combined_and_deduplicated() {
        let headers = combine_header_rows(&[
            vec!["客户".into(), "金额".into()],
            vec!["姓名".into(), "金额".into()],
        ]);
        assert_eq!(headers, vec!["客户 / 姓名", "金额"]);
    }

    #[test]
    fn delimiter_detection_respects_quoted_commas() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("带 空格.csv");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "姓名;说明;金额").unwrap();
        writeln!(file, "张三;\"北京,朝阳\";12").unwrap();
        writeln!(file, "李四;\"上海,浦东\";15").unwrap();
        assert_eq!(detect_delimiter(&path).unwrap(), b';');
        let (tables, warnings) = scan_file(&path).unwrap();
        assert_eq!(tables[0].headers, vec!["姓名", "说明", "金额"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn csv_reader_handles_multiline_quoted_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quoted.csv");
        std::fs::write(&path, "id,note\n1,\"line one\nline two\"\n").unwrap();
        assert_eq!(detect_delimiter(&path).unwrap(), b',');
        let (tables, _) = scan_file(&path).unwrap();
        let table = tables.into_iter().next().unwrap();
        let mut values = Vec::new();
        if let SourceKind::Csv { delimiter } = table.kind {
            for_each_csv_row(
                &path,
                delimiter,
                table.header_row,
                table.header_rows,
                |row| {
                    values.push(row);
                    Ok(())
                },
            )
            .unwrap();
        }
        assert_eq!(values.len(), 1);
        assert_eq!(values[0][1], "line one\nline two");
    }

    #[test]
    fn non_utf8_non_gbk_csv_gets_encoding_warning() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("latin1.csv");
        // "café" 的 é 以 Latin-1 字节 0xE9 写入，既非 UTF-8 也无法按 GBK 完整解码。
        std::fs::write(&path, b"name,value\ncaf\xE9\n").unwrap();
        let (_, warnings) = scan_file(&path).unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("编码可能不是 UTF-8 或 GBK")),
            "warnings: {warnings:?}"
        );
    }

    #[test]
    fn html_table_export_with_xls_extension_is_supported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("网页导出.xls");
        std::fs::write(
            &path,
            "\n\n\n<!DOCTYPE html><html><body><table id=\"2025\"><tr><th>姓名</th><th>金额</th></tr><tr><td>张三</td><td>12</td></tr></table></body></html>",
        )
        .unwrap();

        let (tables, warnings) = scan_file(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].sheet_name, "2025");
        assert_eq!(tables[0].headers, vec!["姓名", "金额"]);
        assert_eq!(tables[0].estimated_rows, 1);

        let preview = crate::inspect::preview_source(&tables[0], 5).unwrap();
        assert_eq!(preview.rows, vec![vec!["张三", "12"]]);
    }

    #[test]
    fn xlsx_content_is_detected_even_with_xls_extension() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("扩展名不准.xls");
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_string(0, 0, "姓名").unwrap();
        worksheet.write_string(1, 0, "李四").unwrap();
        workbook.save(&path).unwrap();

        let (tables, warnings) = scan_file(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(tables[0].headers, vec!["姓名"]);
        let preview = crate::inspect::preview_source(&tables[0], 5).unwrap();
        assert_eq!(preview.rows, vec![vec!["李四"]]);
    }

    #[test]
    fn spreadsheet_xml_with_xlsx_extension_is_supported_as_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("XML导出.xlsx");
        std::fs::write(
            &path,
            r#"<?xml version="1.0"?>
<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet"
 xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <Worksheet ss:Name="2025"><Table>
  <Row><Cell><Data ss:Type="String">姓名</Data></Cell><Cell><Data ss:Type="String">部门</Data></Cell><Cell><Data ss:Type="String">说明</Data></Cell></Row>
  <Row><Cell><Data ss:Type="String">王五</Data></Cell><Cell ss:Index="3"><Data ss:Type="String">审计&amp;财务</Data></Cell></Row>
 </Table></Worksheet>
</Workbook>"#,
        )
        .unwrap();

        let (tables, warnings) = scan_file(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(tables[0].sheet_name, "2025");
        assert_eq!(tables[0].headers, vec!["姓名", "部门", "说明"]);
        let preview = crate::inspect::preview_source(&tables[0], 5).unwrap();
        assert_eq!(preview.rows, vec![vec!["王五", "", "审计&财务"]]);
    }

    #[test]
    fn truncated_xlsx_has_a_clear_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("未完成.xlsx");
        let mut workbook = rust_xlsxwriter::Workbook::new();
        workbook.add_worksheet().write_string(0, 0, "姓名").unwrap();
        workbook.save(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 64);
        std::fs::write(&path, bytes).unwrap();

        let error = scan_file(&path).unwrap_err().to_string();
        assert!(error.contains("XLSX 文件不完整"), "{error}");
    }

    #[test]
    fn xlsx_scan_and_rows_use_streaming_cells() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sparse.xlsx");
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_string(0, 0, "id").unwrap();
        worksheet.write_string(1, 0, "a").unwrap();
        worksheet.write_string(3, 0, "c").unwrap();
        workbook.save(&path).unwrap();

        let (tables, warnings) = scan_file(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].headers, vec!["id"]);
        assert_eq!(tables[0].estimated_rows, 3);

        let mut workbook: Xlsx<BufReader<File>> = open_workbook(&path).unwrap();
        let mut rows = Vec::new();
        for_each_xlsx_row(&mut workbook, "Sheet1", 1, None, |row| {
            rows.push(row.iter().map(Data::to_string).collect::<Vec<_>>());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["a".to_owned()],
                Vec::<String>::new(),
                vec!["c".to_owned()]
            ]
        );
    }

    #[test]
    fn xlsx_scan_ignores_formatted_empty_tail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("formatted-empty-tail.xlsx");
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook.add_worksheet();
        let blank_format =
            rust_xlsxwriter::Format::new().set_background_color(rust_xlsxwriter::Color::Yellow);
        worksheet.write_string(0, 0, "id").unwrap();
        worksheet.write_string(1, 0, "001").unwrap();
        // 模拟 Excel 中“设置过格式但没有内容”的远端单元格。
        worksheet.write_blank(0, 16_000, &blank_format).unwrap();
        worksheet
            .write_blank(50_000, 16_000, &blank_format)
            .unwrap();
        workbook.save(&path).unwrap();

        let (tables, warnings) = scan_file(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].headers, vec!["id"]);
        assert_eq!(tables[0].estimated_rows, 1);

        let mut workbook: Xlsx<BufReader<File>> = open_workbook(&path).unwrap();
        let bounds = for_each_xlsx_row(&mut workbook, "Sheet1", 0, Some(20), |_| Ok(())).unwrap();
        assert_eq!(bounds.actual_end, Some((1, 0)));
        assert_eq!(bounds.actual_columns, Some((0, 0)));
    }
}
