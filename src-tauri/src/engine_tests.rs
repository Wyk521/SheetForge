// 引擎集成测试：真实文件 → 扫描 → 合并 → 读回输出逐格断言。
// 覆盖刁钻场景：编码（BOM/GBK）、长数字保真、空文件、单列、多行表头、
// 日期/布尔单元格、去重、筛选、三种关联、汇总求和、来源列、预检校验等。
use crate::inspect::preflight;
use crate::merge::merge_tables;
use crate::model::{AggregateOp, JoinKind, MergeMode, MergeOptions, SourceTable, TransformOp};
use crate::scan::scan_file;
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// 把 CSV 内容写入临时文件并扫描成 SourceTable。
fn scan_csv(dir: &tempfile::TempDir, name: &str, content: &str) -> SourceTable {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    let (tables, warnings) = scan_file(&path).unwrap();
    assert!(warnings.is_empty(), "扫描 {} 出现警告: {warnings:?}", name);
    tables.into_iter().next().unwrap()
}

/// 生成一个含字符串行的 xlsx 工作簿，返回路径。
fn make_workbook(dir: &tempfile::TempDir, name: &str, rows: &[&[&str]]) -> PathBuf {
    let path = dir.path().join(name);
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    for (row, values) in rows.iter().enumerate() {
        for (col, value) in values.iter().enumerate() {
            worksheet
                .write_string(row as u32, col as u16, *value)
                .unwrap();
        }
    }
    workbook.save(&path).unwrap();
    path
}

/// 扫描工作簿的第一个 Sheet。
fn scan_workbook_sheet(path: &Path, sheet: &str) -> SourceTable {
    let (tables, warnings) = scan_file(path).unwrap();
    assert!(warnings.is_empty());
    tables
        .into_iter()
        .find(|table| table.sheet_name == sheet)
        .unwrap()
}

/// 执行合并并把结果读回为字符串行。
fn merge_and_read(tables: Vec<SourceTable>, options: MergeOptions) -> Vec<Vec<String>> {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("合并结果.xlsx");
    let cancel = AtomicBool::new(false);
    let result = merge_tables(&tables, &options, &output, &|_, _, _| {}, &cancel).unwrap();
    assert!(result.is_some(), "合并应成功");
    let mut workbook = open_workbook_auto(&output).unwrap();
    let range = workbook.worksheet_range("合并结果_001").unwrap();
    range
        .rows()
        .map(|row| row.iter().map(Data::to_string).collect())
        .collect()
}

#[test]
fn csv_bom_and_gbk_are_decoded_and_merged() {
    let dir = tempfile::tempdir().unwrap();
    // UTF-8 带 BOM
    let utf8 = scan_csv(&dir, "utf8.csv", "\u{feff}姓名,金额\n张三,100\n");
    // GBK 编码
    let (gbk_bytes, _, _) = encoding_rs::GBK.encode("李四,200");
    let gbk_path = dir.path().join("gbk.csv");
    let mut content = "姓名,金额\n".as_bytes().to_vec();
    content.extend_from_slice(&gbk_bytes);
    std::fs::write(&gbk_path, content).unwrap();
    let (tables, _) = scan_file(&gbk_path).unwrap();
    let gbk = tables.into_iter().next().unwrap();

    let rows = merge_and_read(
        vec![utf8, gbk],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows[0], vec!["姓名", "金额"], "BOM 不应出现在表头");
    let data: Vec<Vec<String>> = rows[1..].to_vec();
    assert!(data.contains(&vec!["张三".to_owned(), "100".to_owned()]));
    assert!(
        data.contains(&vec!["李四".to_owned(), "200".to_owned()]),
        "GBK 中文应正确解码"
    );
}

#[test]
fn long_numbers_and_leading_zeros_stay_text() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(
        &dir,
        "numbers.csv",
        "编号,金额\n007,12\n9007199254740993,\"1,234\"\n",
    );
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows[1], vec!["007", "12"], "前导零编号必须保持文本");
    assert_eq!(
        rows[2],
        vec!["9007199254740993", "1,234"],
        "超长数字与千分位必须保持原样"
    );
}

#[test]
fn header_only_csv_merges_to_zero_rows() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "empty.csv", "姓名,金额\n");
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows.len(), 1, "只有表头行");
    assert_eq!(rows[0], vec!["姓名", "金额"]);
}

#[test]
fn empty_csv_file_produces_warning_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.csv");
    std::fs::write(&path, "").unwrap();
    // 空文件无法读取表头 → 返回错误，UI 层会把它转为警告而不是崩溃
    assert!(scan_file(&path).is_err(), "空文件应返回错误而非崩溃");
}

#[test]
fn single_column_csv_merges() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "one.csv", "城市\n北京\n上海\n");
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows[0], vec!["城市"]);
    assert_eq!(rows.len(), 3);
}

#[test]
fn multi_row_header_workbook_merges() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_workbook(
        &dir,
        "multi.xlsx",
        &[
            &["报表标题"],
            &["客户", "金额"],
            &["姓名", "金额"],
            &["甲", "10"],
            &["乙", "20"],
        ],
    );
    let mut table = scan_workbook_sheet(&path, "Sheet1");
    table.header_row = 2;
    table.header_rows = 2;
    table.headers = vec!["客户 / 姓名".to_owned(), "金额".to_owned()];
    table.mappings = crate::model::make_default_mappings(&table.headers);
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows[0], vec!["客户 / 姓名", "金额"], "多行表头应合并为一行");
    assert_eq!(rows[1], vec!["甲", "10"]);
    assert_eq!(rows[2], vec!["乙", "20"]);
}

#[test]
fn date_and_boolean_cells_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("types.xlsx");
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.write_string(0, 0, "日期").unwrap();
    worksheet.write_string(0, 1, "启用").unwrap();
    let date = ExcelDateTime::from_ymd(2024, 1, 15).unwrap();
    worksheet
        .write_datetime_with_format(1, 0, &date, &Format::new().set_num_format("yyyy-mm-dd"))
        .unwrap();
    worksheet.write_boolean(1, 1, true).unwrap();
    workbook.save(&path).unwrap();

    let table = scan_workbook_sheet(&path, "Sheet1");
    let rows = merge_and_read(vec![table.clone()], MergeOptions::default());
    assert_eq!(rows[0], vec!["日期", "启用"]);

    // 原始单元格类型检查：日期保持 Excel 日期（序列号 45306 = 2024-01-15），布尔保持布尔
    let output = dir.path().join("out.xlsx");
    let cancel = AtomicBool::new(false);
    merge_tables(
        &[table],
        &MergeOptions::default(),
        &output,
        &|_, _, _| {},
        &cancel,
    )
    .unwrap();
    let mut workbook = open_workbook_auto(&output).unwrap();
    let range = workbook.worksheet_range("合并结果_001").unwrap();
    match range.get_value((1, 0)) {
        Some(Data::DateTime(value)) => assert_eq!(value.as_f64(), 45_306.0),
        other => panic!("日期应保持 DateTime 类型，得到 {other:?}"),
    }
    assert_eq!(range.get_value((1, 1)), Some(&Data::Bool(true)));
}

#[test]
fn consolidate_sums_text_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "sales.csv", "城市,金额\n北京,10\n北京,20\n上海,30\n");
    let mut options = MergeOptions {
        mode: MergeMode::Consolidate,
        key_columns: vec!["城市".to_owned()],
        ..Default::default()
    };
    // 金额列用求和聚合
    options.output_order = vec!["城市".to_owned(), "金额".to_owned()];
    let mut table = table;
    for mapping in &mut table.mappings {
        if mapping.source_name == "金额" {
            mapping.aggregate = AggregateOp::Sum;
        }
    }
    let rows = merge_and_read(vec![table], options);
    assert_eq!(rows[0], vec!["城市", "金额"]);
    assert!(
        rows.contains(&vec!["北京".to_owned(), "30".to_owned()]),
        "文本数字求和应为 30: {rows:?}"
    );
    assert!(rows.contains(&vec!["上海".to_owned(), "30".to_owned()]));
}

#[test]
fn consolidate_large_int_sum_keeps_exact_precision() {
    // 刁钻场景：2^53 + 1，浮点累加会丢一个单位，整数累加须精确得到 9007199254740993
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(
        &dir,
        "big.csv",
        "城市,金额\n北京,9007199254740992\n北京,1\n",
    );
    let mut options = MergeOptions {
        mode: MergeMode::Consolidate,
        key_columns: vec!["城市".to_owned()],
        ..Default::default()
    };
    options.output_order = vec!["城市".to_owned(), "金额".to_owned()];
    let mut table = table;
    for mapping in &mut table.mappings {
        if mapping.source_name == "金额" {
            mapping.aggregate = AggregateOp::Sum;
        }
    }
    let rows = merge_and_read(vec![table], options);
    assert_eq!(
        rows[1],
        vec!["北京".to_owned(), "9007199254740993".to_owned()],
        "超大整数求和必须精确不丢位: {rows:?}"
    );
}

#[test]
fn consolidate_normal_integer_sum_stays_numeric() {
    // 常规整数结果仍是数值单元格（Excel 可参与计算），不因精度保护变文本
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "n.csv", "城市,金额\n北京,10\n北京,20\n");
    let mut options = MergeOptions {
        mode: MergeMode::Consolidate,
        key_columns: vec!["城市".to_owned()],
        ..Default::default()
    };
    options.output_order = vec!["城市".to_owned(), "金额".to_owned()];
    let mut table = table;
    for mapping in &mut table.mappings {
        if mapping.source_name == "金额" {
            mapping.aggregate = AggregateOp::Sum;
        }
    }
    let rows = merge_and_read(vec![table], options);
    assert_eq!(rows[1], vec!["北京".to_owned(), "30".to_owned()]);
}

#[test]
fn consolidate_wide_integer_sum_stays_text_not_scientific() {
    // 刁钻场景：求和结果达 15 位时，绝不能显示成科学计数法，须保持完整数字
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(
        &dir,
        "w.csv",
        "城市,金额\n北京,70000000000000\n北京,70000000000000\n",
    );
    let mut options = MergeOptions {
        mode: MergeMode::Consolidate,
        key_columns: vec!["城市".to_owned()],
        ..Default::default()
    };
    options.output_order = vec!["城市".to_owned(), "金额".to_owned()];
    let mut table = table;
    for mapping in &mut table.mappings {
        if mapping.source_name == "金额" {
            mapping.aggregate = AggregateOp::Sum;
        }
    }
    let rows = merge_and_read(vec![table], options);
    assert_eq!(
        rows[1],
        vec!["北京".to_owned(), "140000000000000".to_owned()],
        "15 位求和结果必须是完整数字文本，不得出现科学计数法: {rows:?}"
    );
}

#[test]
fn dedup_and_filter_work_together() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(
        &dir,
        "dup.csv",
        "姓名,城市\n张三,北京\n张三,北京\n李四,上海\n",
    );
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            deduplicate: true,
            ..Default::default()
        },
    );
    assert_eq!(rows.len(), 3, "整行去重后剩 2 条数据");

    let dir2 = tempfile::tempdir().unwrap();
    let table2 = scan_csv(&dir2, "filter.csv", "姓名,城市\n张三,北京\n李四,上海\n");
    let rows = merge_and_read(
        vec![table2],
        MergeOptions {
            mode: MergeMode::Union,
            filter_column: "城市".to_owned(),
            filter_text: "北".to_owned(),
            ..Default::default()
        },
    );
    assert_eq!(rows.len(), 2, "筛选后只剩北京一行");
    assert_eq!(rows[1], vec!["张三", "北京"]);
}

#[test]
fn join_left_inner_full() {
    let dir = tempfile::tempdir().unwrap();
    let left = scan_csv(&dir, "left.csv", "id,姓名\n1,张三\n2,李四\n");
    let right = scan_csv(&dir, "right.csv", "id,城市\n1,北京\n3,上海\n");

    let run = |kind: JoinKind| {
        merge_and_read(
            vec![left.clone(), right.clone()],
            MergeOptions {
                mode: MergeMode::Join,
                key_columns: vec!["id".to_owned()],
                join_kind: kind,
                ..Default::default()
            },
        )
    };
    let left_rows = run(JoinKind::Left);
    assert_eq!(left_rows.len(), 3, "左关联：2 条数据");
    let inner_rows = run(JoinKind::Inner);
    assert_eq!(inner_rows.len(), 2, "内关联：1 条数据");
    let full_rows = run(JoinKind::Full);
    assert_eq!(full_rows.len(), 4, "全关联：3 条数据");
}

#[test]
fn transforms_and_source_columns() {
    let dir = tempfile::tempdir().unwrap();
    let mut table = scan_csv(&dir, "t.csv", "姓名,城市\n 张三 ,北京\n");
    for mapping in &mut table.mappings {
        if mapping.source_name == "姓名" {
            mapping.transform = TransformOp::Trim;
        }
    }
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            include_source_file: true,
            include_source_sheet: true,
            ..Default::default()
        },
    );
    assert_eq!(rows[1][0], "张三", "去空格变换应生效");
    assert_eq!(rows[1][2], "t.csv", "应附加来源文件列");
    assert_eq!(rows[1][3], "CSV", "应附加来源工作表列");
}

#[test]
fn source_sheet_column_in_the_middle_keeps_data_aligned() {
    // 刁钻场景：用户把「来源工作表」拖到中间（姓名和城市之间）再合并，
    // 不仅表头顺序要正确，每一行的数据也必须落在正确的列上，不能错位。
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "t.csv", "姓名,城市\n张三,北京\n");
    let rows = merge_and_read(
        vec![table],
        MergeOptions {
            mode: MergeMode::Union,
            include_source_sheet: true,
            output_order: vec![
                "姓名".to_owned(),
                "来源工作表".to_owned(),
                "城市".to_owned(),
            ],
            ..Default::default()
        },
    );
    assert_eq!(
        rows[0],
        vec!["姓名", "来源工作表", "城市"],
        "表头顺序应遵循用户拖拽后的位置"
    );
    assert_eq!(
        rows[1],
        vec!["张三", "CSV", "北京"],
        "来源列在中间时，前后列的数据不能错位"
    );
}

#[test]
fn workbook_multiple_sheets_all_merge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two.xlsx");
    let mut workbook = Workbook::new();
    for name in ["华东", "华北"] {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name(name).unwrap();
        worksheet.write_string(0, 0, "城市").unwrap();
        worksheet.write_string(1, 0, format!("{name}市")).unwrap();
    }
    workbook.save(&path).unwrap();
    let (tables, warnings) = scan_file(&path).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(tables.len(), 2, "两个 Sheet 都应被识别");
    let rows = merge_and_read(
        tables,
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(rows.len(), 3, "表头 + 2 行数据");
}

#[test]
fn preflight_catches_missing_filter_column() {
    let dir = tempfile::tempdir().unwrap();
    let table = scan_csv(&dir, "a.csv", "姓名\n张三\n");
    let options = MergeOptions {
        mode: MergeMode::Union,
        filter_column: "不存在的列".to_owned(),
        filter_text: "x".to_owned(),
        ..Default::default()
    };
    let issues = preflight(&[table], &options);
    assert!(
        issues.iter().any(|issue| issue.title == "筛选字段不存在"),
        "应报出筛选字段不存在的错误: {issues:?}"
    );
}

#[test]
fn union_merges_intersection_and_manual() {
    let dir = tempfile::tempdir().unwrap();
    let a = scan_csv(&dir, "a.csv", "姓名,年龄\n张三,30\n");
    let b = scan_csv(&dir, "b.csv", "姓名,城市\n张三,北京\n");

    let union = merge_and_read(
        vec![a.clone(), b.clone()],
        MergeOptions {
            mode: MergeMode::Union,
            ..Default::default()
        },
    );
    assert_eq!(union[0], vec!["姓名", "年龄", "城市"]);

    let intersection = merge_and_read(
        vec![a.clone(), b.clone()],
        MergeOptions {
            mode: MergeMode::Intersection,
            ..Default::default()
        },
    );
    assert_eq!(intersection[0], vec!["姓名"]);

    let mut manual_a = a.clone();
    let mut manual_b = b.clone();
    manual_a.mappings[0].target_name = "人员".to_owned();
    manual_b.mappings[0].target_name = "人员".to_owned();
    let manual = merge_and_read(
        vec![manual_a, manual_b],
        MergeOptions {
            mode: MergeMode::Manual,
            ..Default::default()
        },
    );
    // 手动映射包含所有启用的映射列；两张表的“姓名”都映射到了“人员”
    assert_eq!(manual[0], vec!["人员", "年龄", "城市"]);
    assert_eq!(manual.len(), 3, "表头 + 2 行数据");
    assert!(manual.iter().any(|row| row[0] == "张三"));
}

#[test]
fn batch_rename_same_field_across_tables() {
    let dir = tempfile::tempdir().unwrap();
    let a = scan_csv(&dir, "a.csv", "姓名,金额\n张三,100\n");
    let b = scan_csv(&dir, "b.csv", "姓名,金额\n李四,200\n");
    let c = scan_csv(&dir, "c.csv", "姓名,金额,备注\n王五,300,ok\n");
    let d = scan_csv(&dir, "d.csv", "姓名\n赵六\n");

    // 模拟「按字段改表」的批量动作：把所有表中“金额”这个来源字段全部映射到“合同金额”
    // （不含该字段的表不受影响）
    let tables: Vec<SourceTable> = vec![a, b, c, d]
        .into_iter()
        .map(|mut table| {
            for mapping in &mut table.mappings {
                if mapping.source_name == "金额" {
                    mapping.target_name = "合同金额".to_owned();
                }
            }
            table
        })
        .collect();

    let rows = merge_and_read(
        tables,
        MergeOptions {
            mode: MergeMode::Manual,
            ..Default::default()
        },
    );
    assert_eq!(
        rows[0],
        vec!["姓名", "合同金额", "备注"],
        "同名来源字段应统一改名"
    );
    let data: Vec<Vec<String>> = rows[1..].to_vec();
    let amount_col = rows[0].iter().position(|h| h == "合同金额").unwrap();
    assert!(data
        .iter()
        .any(|row| row[0] == "张三" && row.get(amount_col).map(String::as_str) == Some("100")));
    assert!(data
        .iter()
        .any(|row| row[0] == "李四" && row.get(amount_col).map(String::as_str) == Some("200")));
    assert!(data
        .iter()
        .any(|row| row[0] == "王五" && row.get(amount_col).map(String::as_str) == Some("300")));
    // 不含“金额”字段的表不改动、也不补空列
    let zhao = data.iter().find(|row| row[0] == "赵六").unwrap();
    assert_eq!(zhao.get(amount_col).map(String::as_str).unwrap_or(""), "");
}
