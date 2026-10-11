//! Explicit local benchmarks: generate once, then run each merge in a fresh process.
use crate::{merge::merge_tables, model::MergeOptions, scan::scan_file};
use std::{fs::File, io::Write, path::PathBuf, sync::atomic::AtomicBool};

fn directory() -> PathBuf {
    std::env::var_os("SHEETFORGE_BENCH_DIR")
        .map(PathBuf::from)
        .expect("set SHEETFORGE_BENCH_DIR to an isolated benchmark directory")
}

#[test]
#[ignore = "generates large benchmark fixtures"]
fn generate_fixtures() {
    let directory = directory();
    std::fs::create_dir_all(&directory).unwrap();
    let mut csv = std::io::BufWriter::new(File::create(directory.join("million.csv")).unwrap());
    writeln!(csv, "编号,客户,城市,金额,备注,部门,日期,状态").unwrap();
    for row in 0..1_000_000 {
        writeln!(
            csv,
            "{row:010},客户{row},上海,123.45,测试,业务部,2026-10-11,完成"
        )
        .unwrap();
    }
    csv.flush().unwrap();
    let mut workbook = rust_xlsxwriter::Workbook::new();
    for sheet in 0..10 {
        let worksheet = workbook.add_worksheet_with_constant_memory();
        worksheet.set_name(format!("Sheet{sheet}")).unwrap();
        for (column, name) in [
            "编号", "客户", "城市", "金额", "备注", "部门", "日期", "状态",
        ]
        .iter()
        .enumerate()
        {
            worksheet.write_string(0, column as u16, *name).unwrap();
        }
        for row in 1..=10_000 {
            for column in 0..8 {
                worksheet
                    .write_string(row, column, format!("{sheet}-{row}-{column}"))
                    .unwrap();
            }
        }
    }
    workbook.save(directory.join("multi.xlsx")).unwrap();
}

#[test]
#[ignore = "run under an external time/RSS measurement tool"]
fn merge_fixture() {
    let directory = directory();
    let kind = std::env::var("SHEETFORGE_BENCH_KIND").expect("csv or xlsx");
    let (name, expected_rows) = match kind.as_str() {
        "csv" => ("million.csv", 1_000_000),
        "xlsx" => ("multi.xlsx", 100_000),
        _ => panic!("unsupported fixture"),
    };
    let (tables, warnings) = scan_file(&directory.join(name)).unwrap();
    assert!(warnings.is_empty());
    let summary = merge_tables(
        &tables,
        &MergeOptions::default(),
        &directory.join(format!("{kind}-output.xlsx")),
        &|_, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(summary.rows, expected_rows);
    assert_eq!(summary.truncated_cells, 0);
}
