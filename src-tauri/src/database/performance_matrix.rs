//! Explicit synthetic performance matrix; runs only when requested by the batch script.
use crate::{
    inspect,
    merge::{self, CellValue},
    model::{MergeOptions, SourceTable, TransformOp},
    scan,
};
use anyhow::Result;
use calamine::Reader;
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

#[derive(Clone, Deserialize)]
struct Case {
    name: String,
    kind: String,
    rows: u32,
    cols: usize,
    #[serde(default)]
    gbk: bool,
    #[serde(default)]
    shared: bool,
    #[serde(default = "one")]
    sheets: usize,
    #[serde(default = "one")]
    files: usize,
    #[serde(default)]
    offset: u16,
    #[serde(default)]
    title: bool,
    #[serde(default)]
    heterogeneous: bool,
}
fn one() -> usize {
    1
}
fn effective(r: u32) -> u32 {
    if r.is_multiple_of(10) {
        r - 1
    } else {
        r
    }
}
fn header(c: &Case, f: usize, col: usize) -> String {
    let role = [
        "编号",
        "客户类别",
        "负数",
        "金额",
        "日期时间",
        "布尔值",
        "多行备注",
        "空值",
        "状态",
        "清洗文本",
        "长备注",
        "科学计数",
    ][col % 12];
    if c.heterogeneous {
        let mut x = (f * c.cols + col + 1) as u64;
        x = x.wrapping_add(0x9e3779b97f4a7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        format!("H_{:016x}_{role}", x ^ (x >> 31))
    } else {
        format!("字段{col:03}_{role}")
    }
}
fn text_value(c: &Case, f: usize, s: usize, r: u32, col: usize) -> String {
    let r = effective(r);
    match col % 12 {
        0 => format!("00{f:03}{s:02}{col:03}{r:012}"),
        1 => format!("客户类别{:02}", r % 17),
        2 if c.name.contains("numeric_edges") => [
            "-1.7976931348623157e308",
            "2.2250738585072014e-308",
            "5e-324",
            "9007199254740991",
            "9007199254740993",
            "-0",
            "123456789012345678901234567890123456",
        ][r as usize % 7]
            .into(),
        2 => format!("-{}", r as u64 * 17 + col as u64),
        3 => format!("{}.125", r as u64 * 1000 + col as u64),
        4 => format!("2026-10-{:02} 12:34:56.125", r % 28 + 1),
        5 => if r.is_multiple_of(2) { "true" } else { "false" }.into(),
        6 => format!(
            "中文,逗号\"引号\"\n第二行\t反斜杠\\N-{}-{}",
            r,
            if c.gbk { "测试" } else { "🙂" }
        ),
        7 => {
            if r.is_multiple_of(3) {
                String::new()
            } else {
                format!("可选{r}")
            }
        }
        8 => format!("重复状态{}", r % 5),
        9 => format!("  abC中文{r}  "),
        10 => {
            if r.is_multiple_of(137) {
                format!("长文本{}-{r}", "甲乙丙丁".repeat(128))
            } else {
                format!("备注{f}-{s}-{r}-{col}")
            }
        }
        _ => format!("{}e-8", r % 997 + 1),
    }
}
fn sources(dir: &Path, c: &Case) -> Vec<PathBuf> {
    (0..c.files)
        .map(|f| dir.join("sources").join(format!("input{f:03}.{}", c.kind)))
        .collect()
}
fn generate(dir: &Path, c: &Case) -> Result<Value> {
    fs::create_dir_all(dir.join("sources"))?;
    let datetime_format = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss.000");
    let mut save_ms = 0.0;
    for (f, path) in sources(dir, c).iter().enumerate() {
        if c.kind == "csv" {
            let mut out = BufWriter::new(File::create(path)?);
            for r in 0..=c.rows {
                let vals = (0..c.cols)
                    .map(|col| {
                        if r == 0 {
                            header(c, f, col)
                        } else {
                            text_value(c, f, 0, r, col)
                        }
                    })
                    .collect::<Vec<_>>();
                let mut record = csv::Writer::from_writer(Vec::new());
                record.write_record(vals)?;
                let bytes = record.into_inner()?;
                if c.gbk {
                    let text = std::str::from_utf8(&bytes)?;
                    let (encoded, _, errors) = encoding_rs::GBK.encode(text);
                    assert!(!errors);
                    out.write_all(&encoded)?;
                } else {
                    out.write_all(&bytes)?;
                }
            }
            out.flush()?;
        } else {
            let mut wb = Workbook::new();
            if let Ok(level) = std::env::var("SF_COMPRESSION") {
                wb.set_zip_compression_level(level.parse::<u8>()?);
            }
            for s in 0..c.sheets {
                let ws = if c.shared {
                    wb.add_worksheet()
                } else {
                    wb.add_worksheet_with_constant_memory()?
                };
                ws.set_name(format!("数据{s:02}"))?;
                let hr = if c.title { 1 } else { 0 };
                if c.title {
                    ws.write_string(0, c.offset + 4, "年度业务汇总总标题")?;
                }
                for col in 0..c.cols {
                    ws.write_string(hr, col as u16 + c.offset, header(c, f, col))?;
                }
                for r in 1..=c.rows {
                    let er = effective(r);
                    for col in 0..c.cols {
                        let cr = col as u16 + c.offset;
                        let rr = r + hr;
                        match col % 12 {
                            2 if c.name.contains("numeric_edges") => {
                                ws.write_number(
                                    rr,
                                    cr,
                                    [
                                        -1.234567891e100,
                                        1e-100,
                                        9007199254740991.0,
                                        9007199254740992.0,
                                        -0.0,
                                    ][er as usize % 5],
                                )?;
                            }
                            2 => {
                                ws.write_number(rr, cr, -((er as u64 * 17 + col as u64) as f64))?;
                            }
                            3 => {
                                ws.write_number(
                                    rr,
                                    cr,
                                    (er as u64 * 1000 + col as u64) as f64 + 0.125,
                                )?;
                            }
                            4 => {
                                let dt = ExcelDateTime::from_ymd(2026, 10, (er % 28 + 1) as u8)?
                                    .and_hms(12, 34, 56.125)?;
                                ws.write_datetime_with_format(rr, cr, &dt, &datetime_format)?;
                            }
                            5 => {
                                ws.write_boolean(rr, cr, er.is_multiple_of(2))?;
                            }
                            7 if er.is_multiple_of(3) => {}
                            11 => {
                                ws.write_number(rr, cr, (er % 997 + 1) as f64 * 1e-8)?;
                            }
                            _ => {
                                ws.write_string(rr, cr, text_value(c, f, s, r, col))?;
                            }
                        }
                    }
                }
            }
            let at = Instant::now();
            wb.save(path)?;
            save_ms += at.elapsed().as_secs_f64() * 1000.0;
        }
    }
    let bytes: u64 = sources(dir, c)
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .sum();
    Ok(json!({"input_bytes":bytes,"save_ms":save_ms}))
}
fn hash_row(h: &mut Sha256, row: impl IntoIterator<Item = String>) {
    for value in row {
        h.update((value.len() as u64).to_le_bytes());
        h.update(value.as_bytes());
    }
    h.update([255]);
}
fn preview_hash(rows: &[Vec<String>]) -> String {
    let mut h = Sha256::new();
    for r in rows {
        hash_row(&mut h, r.iter().cloned());
    }
    hex::encode(h.finalize())
}
#[test]
#[ignore = "explicit isolated performance audit"]
fn run_stage() {
    run().unwrap();
}
fn run() -> Result<()> {
    let base = PathBuf::from(std::env::var("SF_REVIEW_DIR")?);
    let name = std::env::var("SF_CASE")?;
    let stage = std::env::var("SF_STAGE")?;
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(base.join("cases.json"))?)?;
    let c = cases.into_iter().find(|c| c.name == name).unwrap();
    let dir = base.join("fixtures").join(&name);
    fs::create_dir_all(&dir)?;
    let mut tables: Vec<SourceTable> = if stage == "generate" || stage == "scan" {
        Vec::new()
    } else {
        serde_json::from_slice(&fs::read(dir.join("tables.json"))?)?
    };
    let mut opts = MergeOptions::default();
    if stage.contains("filter") {
        opts.filter_column = header(
            &Case {
                heterogeneous: false,
                ..c.clone()
            },
            0,
            1,
        );
        opts.filter_text = "客户类别03".into();
    }
    if stage.contains("dedup") {
        opts.deduplicate = true;
    }
    if stage.contains("transform") {
        for t in &mut tables {
            for m in &mut t.mappings {
                if m.source_index % 12 == 9 {
                    m.transform = TransformOp::Uppercase;
                }
            }
        }
    }
    let expected = c.rows as u64 * c.sheets as u64 * c.files as u64;
    // Simulate the normal app path: scanning has already validated origin/encoding.
    // Setup is outside timed preview stages, but remains in process peak RSS.
    if stage.ends_with("_warm") {
        for path in sources(&dir, &c) {
            scan::scan_file(&path)?;
        }
    }
    if stage.contains("filter_late") {
        opts.filter_column = header(
            &Case {
                heterogeneous: false,
                ..c.clone()
            },
            0,
            (0..c.cols).rev().find(|col| col % 12 == 9).unwrap_or(1),
        );
        opts.filter_text = if c.cols > 9 {
            "abC".into()
        } else {
            "客户类别".into()
        };
    }
    let stage = stage.trim_end_matches("_warm").to_owned();
    if stage == "source_preview_cached" {
        inspect::preview_source(&tables[0], 30)?;
    }
    let at = Instant::now();
    let result: Value = match stage.as_str() {
        "generate" => generate(&dir, &c)?,
        "discover" => {
            let paths = scan::collect_folder(&dir.join("sources"));
            assert_eq!(paths.len(), c.files);
            json!({"files":paths.len()})
        }
        "scan" => {
            let paths = sources(&dir, &c);
            let found = std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for worker in 0..c.files.min(4) {
                    let paths = &paths;
                    handles.push(scope.spawn(move || {
                        paths
                            .iter()
                            .skip(worker)
                            .step_by(c.files.min(4))
                            .map(|p| scan::scan_file(p))
                            .collect::<Vec<_>>()
                    }));
                }
                handles
                    .into_iter()
                    .flat_map(|h| h.join().unwrap())
                    .collect::<Vec<_>>()
            });
            let scan_ms = at.elapsed().as_secs_f64() * 1000.;
            let mut warnings = Vec::new();
            for r in found {
                let (mut ts, ws) = r?;
                tables.append(&mut ts);
                warnings.extend(ws);
            }
            tables.sort_by_key(|t| (t.path.clone(), t.sheet_name.clone()));
            assert_eq!(tables.len(), c.files * c.sheets);
            for t in &mut tables {
                assert_eq!(t.headers.len(), c.cols, "headers {}", t.display_name());
                assert_eq!(t.header_row, if c.title { 2 } else { 1 }, "header row");
                if c.heterogeneous {
                    for m in &mut t.mappings {
                        m.target_name = header(
                            &Case {
                                heterogeneous: false,
                                ..c.clone()
                            },
                            0,
                            m.source_index,
                        );
                    }
                }
            }
            fs::write(dir.join("tables.json"), serde_json::to_vec(&tables)?)?;
            json!({"measured_ms":scan_ms,"tables":tables.len(),"estimated_rows":tables.iter().map(|t|t.estimated_rows).sum::<u64>(),"warnings":warnings})
        }
        "source_preview" | "source_preview_cached" => {
            let p = inspect::preview_source(&tables[0], 30)?;
            assert_eq!(p.rows.len(), 30.min(c.rows as usize));
            assert_eq!(p.rows[0][0], text_value(&c, 0, 0, 1, 0));
            json!({"rows":p.rows.len(),"hash":preview_hash(&p.rows)})
        }
        "merged_preview" => {
            let p = inspect::preview_merged(&tables, &opts, 5)?;
            assert_eq!(p.groups.len(), tables.len());
            json!({"groups":p.groups.len(),"rows":p.groups.iter().map(|g|g.rows.len()).sum::<usize>()})
        }
        "check_xlsx" | "check_pg" => {
            let issues = inspect::preflight_for_destination(&tables, &opts, stage == "check_pg");
            let errors = issues
                .iter()
                .filter(|i| i.level == inspect::IssueLevel::Error)
                .count();
            assert_eq!(errors, 0, "{issues:?}");
            json!({"issues":issues.len()})
        }
        "merge_xlsx" | "merge_xlsx_fast" => {
            if stage == "merge_xlsx_fast" {
                let mut value = serde_json::to_value(&opts)?;
                value["fast_xlsx"] = json!(true);
                opts = serde_json::from_value(value)?;
            }
            let summary = merge::merge_tables(
                &tables,
                &opts,
                &dir.join("output.xlsx"),
                &|_, _, _| {},
                &AtomicBool::new(false),
            )?
            .unwrap();
            assert_eq!(summary.rows, expected);
            assert_eq!(summary.truncated_cells, 0);
            json!({"rows":summary.rows,"sheets":summary.sheets,"output_bytes":fs::metadata(dir.join("output.xlsx"))?.len()})
        }
        "verify_output" => {
            let expected_hash: Value =
                serde_json::from_slice(&fs::read(dir.join("source_hash.json"))?)?;
            let mut wb = scan::open_xlsx_workbook(&dir.join("output.xlsx"))?;
            let names = wb.sheet_names().to_owned();
            let mut h = Sha256::new();
            let mut count = 0u64;
            for sheet in names {
                scan::for_each_xlsx_row(&mut wb, &sheet, 1, None, |r| {
                    let mut values = vec![String::new(); c.cols];
                    for (i, v) in r.iter().enumerate().take(c.cols) {
                        values[i] = CellValue::from_calamine(v)
                            .copy_text(false)
                            .unwrap_or_default()
                            .into_owned();
                    }
                    hash_row(&mut h, values);
                    count += 1;
                    Ok(())
                })?;
            }
            let hash = hex::encode(h.finalize());
            assert_eq!(count, expected);
            assert_eq!(
                hash,
                expected_hash["hash"].as_str().unwrap(),
                "output content hash"
            );
            json!({"rows":count,"hash":hash,"verified":true})
        }
        "pg_binary" | "pg_text" => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            runtime.block_on(async{
                let profile=crate::pg_import::config::ConnectionProfile{host:"127.0.0.1".into(),port:std::env::var("SF_PG_PORT")?.parse()?,database:"postgres".into(),user:"postgres".into(),sslmode:"disable".into()};
                let request=super::DatabaseImportRequest{profile_name:String::new(),password:None,remember_password:false,schema:"public".into(),table:"sf_performance".into(),if_exists:"replace".into(),copy_format:stage.trim_start_matches("pg_").into(),table_persistence:"logged".into(),empty_as_null:true,fast_mode:false};
                let out=super::engine::import_database(tables.clone(),opts.clone(),request,profile.clone(),None,Arc::new(AtomicBool::new(false)),&|_,_,_|{}).await?.unwrap();
                let elapsed_ms=at.elapsed().as_secs_f64()*1000.;assert_eq!(out.rows,expected);
                let (client,_)=crate::pg_import::postgres::connection::connect(&profile,None).await?;let count:i64=client.query_one("SELECT count(*) FROM public.sf_performance",&[]).await?.get(0);assert_eq!(count as u64,expected);
                let field=crate::model::build_output_plan(&tables,&opts).headers[0].clone();let ids=crate::pg_import::schema::identifier::map_identifiers(&[field]);
                // Full row count and long identifier are checked after COPY, outside measured import time.
                let sql=format!("SELECT {} FROM public.sf_performance LIMIT 1",crate::pg_import::schema::identifier::quote_identifier(&ids[0].database));
                let id:String=client.query_one(&sql,&[]).await?.get(0);assert_eq!(id,text_value(&c,0,0,1,0));
                Ok::<_,anyhow::Error>(json!({"measured_ms":elapsed_ms,"rows":out.rows,"bytes":out.bytes,"batches":out.batches,"verified":true}))
            })?
        }
        _ if stage.starts_with("stream") => {
            let mut count = 0;
            let mut h = Sha256::new();
            merge::stream_merged_cells(
                &tables,
                &opts,
                &|_, _, _| {},
                &AtomicBool::new(false),
                |row| {
                    count += 1;
                    {
                        hash_row(
                            &mut h,
                            row.iter()
                                .map(|v| v.copy_text(false).unwrap_or_default().into_owned()),
                        );
                    }
                    Ok(())
                },
            )?
            .unwrap();
            let expected_rows = if stage.contains("dedup") {
                (c.rows - c.rows / 10) as u64 * c.sheets as u64 * c.files as u64
            } else if stage.contains("filter_late") {
                expected
            } else if stage.contains("filter") {
                (1..=c.rows).filter(|r| effective(*r) % 17 == 3).count() as u64
                    * c.sheets as u64
                    * c.files as u64
            } else {
                expected
            };
            assert_eq!(count, expected_rows);
            let value = json!({"rows":count,"hash":hex::encode(h.finalize())});
            if stage == "stream_hash" {
                fs::write(dir.join("source_hash.json"), serde_json::to_vec(&value)?)?;
            }
            value
        }
        _ => anyhow::bail!("unknown stage {stage}"),
    };
    let elapsed = result
        .get("measured_ms")
        .and_then(Value::as_f64)
        .unwrap_or(at.elapsed().as_secs_f64() * 1000.);
    println!(
        "RESULT {}",
        json!({"case":name,"stage":stage,"ms":elapsed,"data":result})
    );
    Ok(())
}
