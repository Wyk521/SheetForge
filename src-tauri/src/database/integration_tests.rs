//! Explicit PostgreSQL 17 tests. Missing configuration is a failure, never a skip.
use super::{engine::import_database, DatabaseImportRequest};
use crate::{
    merge::{merge_tables, CellValue},
    model::{MergeMode, MergeOptions, SourceTable, TransformOp},
    pg_import::{config::ConnectionProfile, postgres::connection},
    scan::scan_file,
};
use calamine::{open_workbook_auto, Data, Reader};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(0);

struct Database {
    profile: ConnectionProfile,
    password: String,
    schema: String,
    client: tokio_postgres::Client,
}

impl Database {
    async fn new() -> Self {
        let required = |name| {
            std::env::var(name)
                .expect("PostgreSQL integration environment is required; see tests/README.md")
        };
        let profile = ConnectionProfile {
            host: "127.0.0.1".into(),
            port: required("SHEETFORGE_TEST_POSTGRES_PORT").parse().unwrap(),
            database: required("SHEETFORGE_TEST_POSTGRES_DATABASE"),
            user: required("SHEETFORGE_TEST_POSTGRES_USER"),
            sslmode: "disable".into(),
        };
        let password = required("SHEETFORGE_TEST_POSTGRES_PASSWORD");
        let (client, _) = connection::connect(&profile, Some(&password))
            .await
            .unwrap();
        let version: String = client
            .query_one("SHOW server_version_num", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            version.parse::<u32>().unwrap() / 10_000,
            17,
            "test PostgreSQL 17 explicitly"
        );
        let schema = format!(
            "sheetforge_dt_{}_{}",
            std::process::id(),
            NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
        );
        client
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        Self {
            profile,
            password,
            schema,
            client,
        }
    }

    fn request(&self, table: &str, format: &str, if_exists: &str) -> DatabaseImportRequest {
        DatabaseImportRequest {
            profile_name: String::new(),
            password: None,
            remember_password: false,
            schema: self.schema.clone(),
            table: table.into(),
            if_exists: if_exists.into(),
            copy_format: format.into(),
            table_persistence: "logged".into(),
            empty_as_null: true,
            fast_mode: false,
        }
    }

    async fn import(
        &self,
        tables: Vec<SourceTable>,
        options: MergeOptions,
        request: DatabaseImportRequest,
    ) -> anyhow::Result<Option<super::DatabaseImportFinishedDto>> {
        import_database(
            tables,
            options,
            request,
            self.profile.clone(),
            Some(self.password.clone()),
            Arc::new(AtomicBool::new(false)),
            &|_, _, _| {},
        )
        .await
    }

    async fn cleanup(self) {
        self.client
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await
            .unwrap();
    }
}

fn run(future: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(future);
}

fn source(directory: &tempfile::TempDir, name: &str, content: &str) -> SourceTable {
    let path = directory.path().join(name);
    std::fs::write(&path, content).unwrap();
    scan_file(&path).unwrap().0.remove(0)
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_copy_formats_preserve_null_empty_and_escaping() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let mut table = source(
            &dir,
            "input.csv",
            "id,note,missing\n001,\"中文\t反斜杠\\N\n第二行\",\n002,,\n",
        );
        // A CSV source can have an empty sheet label. It becomes an explicit Text("")
        // provenance cell; blank input cells remain Empty in the shared row mapper.
        table.sheet_name.clear();
        let options = MergeOptions {
            include_source_sheet: true,
            ..Default::default()
        };
        for format in ["binary", "text"] {
            for nulls in [true, false] {
                let name = format!("copy_{format}_{nulls}");
                let mut request = db.request(&name, format, "abort");
                request.empty_as_null = nulls;
                request.table_persistence = "unlogged".into();
                request.fast_mode = true;
                let summary = db
                    .import(vec![table.clone()], options.clone(), request)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(summary.rows, 2);
                assert!(summary.copy_freeze);
                assert!(summary.bytes > 0 && summary.batches > 0);
                let rows = db
                    .client
                    .query(
                        &format!(
                            "SELECT id, note, missing, \"来源工作表\" FROM {}.{name} ORDER BY id",
                            db.schema
                        ),
                        &[],
                    )
                    .await
                    .unwrap();
                assert_eq!(rows[0].get::<_, String>(0), "001");
                assert_eq!(rows[0].get::<_, String>(1), "中文\t反斜杠\\N\n第二行");
                // The shared mapper represents blank source cells as missing, preserving existing NULL semantics.
                let empty: Option<String> = None;
                assert_eq!(rows[0].get::<_, Option<String>>(2), empty);
                assert_eq!(rows[1].get::<_, Option<String>>(1), empty);
                for row in &rows {
                    assert_eq!(
                        row.get::<_, Option<String>>(3),
                        if nulls { None } else { Some(String::new()) },
                        "empty text obeys empty_as_null; missing input cells are always NULL"
                    );
                }
            }
        }
        db.cleanup().await;
    });
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_typed_workbooks_match_xlsx_in_both_modes() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let mut tables = Vec::new();
        for (name, id) in [("a", "001"), ("b", "002")] {
            let path = dir.path().join(format!("{name}.xlsx"));
            let mut workbook = Workbook::new();
            let sheet = workbook.add_worksheet();
            sheet.set_name("数据").unwrap();
            for (column, header) in [
                "id", "name", "amount", "date", "time", "flag", "missing", "long_id", name,
            ]
            .iter()
            .enumerate()
            {
                sheet.write_string(0, column as u16, *header).unwrap();
            }
            sheet.write_string(1, 0, id).unwrap();
            sheet.write_string(1, 1, " Alice ").unwrap();
            sheet.write_number(1, 2, 12.5).unwrap();
            sheet
                .write_datetime_with_format(
                    1,
                    3,
                    ExcelDateTime::from_ymd(2024, 1, 15).unwrap(),
                    &Format::new().set_num_format("yyyy-mm-dd"),
                )
                .unwrap();
            sheet
                .write_datetime_with_format(
                    1,
                    4,
                    ExcelDateTime::from_hms(8, 30, 15.0).unwrap(),
                    &Format::new().set_num_format("hh:mm:ss"),
                )
                .unwrap();
            sheet.write_boolean(1, 5, true).unwrap();
            sheet.write_string(1, 7, "9007199254740993").unwrap();
            sheet.write_string(1, 8, name).unwrap();
            workbook.save(&path).unwrap();
            let mut table = scan_file(&path).unwrap().0.remove(0);
            // These manual rules must be ignored by Intersection.
            table.mappings[1].transform = TransformOp::Uppercase;
            table.mappings[8].enabled = false;
            tables.push(table);
        }
        for (mode, label) in [
            (MergeMode::Manual, "manual"),
            (MergeMode::Intersection, "intersection"),
        ] {
            let options = MergeOptions {
                mode,
                include_source_file: true,
                include_source_sheet: true,
                output_order: [
                    "name",
                    "id",
                    "amount",
                    "date",
                    "time",
                    "flag",
                    "missing",
                    "long_id",
                    "来源文件",
                    "来源工作表",
                ]
                .map(String::from)
                .to_vec(),
                ..Default::default()
            };
            let path = dir.path().join(format!("out_{label}.xlsx"));
            merge_tables(
                &tables,
                &options,
                &path,
                &|_, _, _| {},
                &AtomicBool::new(false),
            )
            .unwrap()
            .unwrap();
            let range = open_workbook_auto(&path)
                .unwrap()
                .worksheet_range("合并结果_001")
                .unwrap();
            let expected_name = if mode == MergeMode::Manual {
                "ALICE"
            } else {
                " Alice "
            };
            for (index, row) in range.rows().skip(1).enumerate() {
                assert_eq!(row[0], Data::String(expected_name.into()));
                assert_eq!(row[1], Data::String(format!("{:03}", index + 1)));
                assert!(matches!(row[2], Data::Float(v) if v == 12.5));
                assert!(matches!(row[3], Data::DateTime(_)));
                assert!(matches!(row[4], Data::DateTime(_)));
                assert_eq!(row[5], Data::Bool(true));
                assert_eq!(row[6], Data::Empty);
                assert_eq!(row[7], Data::String("9007199254740993".into()));
            }
            let expected = range
                .rows()
                .skip(1)
                .map(|row| {
                    row.iter()
                        .map(|value| {
                            CellValue::from_calamine(value)
                                .copy_text(false)
                                .map(|v| v.into_owned())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(expected[0][3].as_deref(), Some("2024-01-15"));
            assert_eq!(expected[0][4].as_deref(), Some("08:30:15"));
            for format in ["binary", "text"] {
                let name = format!("{label}_{format}");
                let summary = db
                    .import(
                        tables.clone(),
                        options.clone(),
                        db.request(&name, format, "abort"),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(summary.rows, 2);
                let rows = db
                    .client
                    .query(
                        &format!("SELECT * FROM {}.{name} ORDER BY id", db.schema),
                        &[],
                    )
                    .await
                    .unwrap();
                let actual = rows
                    .iter()
                    .map(|row| {
                        (0..row.len())
                            .map(|column| row.get::<_, Option<String>>(column))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual, expected,
                    "{label} {format}: typed file semantics must match XLSX"
                );
            }
        }
        db.cleanup().await;
    });
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_processed_rows_match_xlsx_and_append() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let mut a = source(&dir, "a.csv", "id,name\n001, a \n001, a \n002,b\n");
        let mut b = source(&dir, "b.csv", "id,name\n003, c \n");
        for table in [&mut a, &mut b] {
            table.mappings[1].transform = TransformOp::Uppercase;
        }
        let tables = vec![a, b];
        let options = MergeOptions {
            deduplicate: true,
            filter_column: "name".into(),
            filter_text: "B".into(),
            filter_exclude: true,
            include_source_file: true,
            include_source_sheet: true,
            output_order: vec![
                "name".into(),
                "来源工作表".into(),
                "id".into(),
                "来源文件".into(),
            ],
            ..Default::default()
        };
        let path = dir.path().join("out.xlsx");
        merge_tables(
            &tables,
            &options,
            &path,
            &|_, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        let mut workbook = open_workbook_auto(&path).unwrap();
        let range = workbook.worksheet_range("合并结果_001").unwrap();
        let expected = range
            .rows()
            .skip(1)
            .map(|row| row.iter().map(Data::to_string).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        for format in ["binary", "text"] {
            let summary = db
                .import(
                    tables.clone(),
                    options.clone(),
                    db.request(format, format, "abort"),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(summary.rows, expected.len() as u64);
            let rows = db
                .client
                .query(
                    &format!("SELECT * FROM {}.{format} ORDER BY id", db.schema),
                    &[],
                )
                .await
                .unwrap();
            let actual = rows
                .iter()
                .map(|row| {
                    (0..row.len())
                        .map(|column| row.get::<_, String>(column))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "both adapters consume identical processed cells"
            );
            let appended = db
                .import(
                    tables.clone(),
                    options.clone(),
                    db.request(format, format, "append"),
                )
                .await
                .unwrap()
                .unwrap();
            assert!(!appended.copy_freeze);
            let count: i64 = db
                .client
                .query_one(&format!("SELECT count(*) FROM {}.{format}", db.schema), &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(count, 2 * expected.len() as i64);
        }
        db.cleanup().await;
    });
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_append_validation_and_truncate_rollback() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let table = source(&dir, "input.csv", "id,note\n001,hello\n");
        for (name, definition) in [
            ("wrong_type", "id INTEGER, note TEXT"),
            ("missing", "id TEXT"),
            ("required", "id TEXT, note TEXT, extra TEXT NOT NULL"),
        ] {
            db.client
                .batch_execute(&format!(
                    "CREATE TABLE {}.{name} ({definition}); INSERT INTO {}.{name} VALUES ({})",
                    db.schema,
                    db.schema,
                    if name == "wrong_type" {
                        "99, 'old'"
                    } else if name == "missing" {
                        "'old'"
                    } else {
                        "'old', 'old', 'old'"
                    }
                ))
                .await
                .unwrap();
            for mode in ["append", "truncate"] {
                let error = db
                    .import(
                        vec![table.clone()],
                        MergeOptions::default(),
                        db.request(name, "binary", mode),
                    )
                    .await
                    .err()
                    .expect("invalid append must fail");
                assert!(error.to_string().contains("append"), "{error:#}");
                let count: i64 = db
                    .client
                    .query_one(&format!("SELECT count(*) FROM {}.{name}", db.schema), &[])
                    .await
                    .unwrap()
                    .get(0);
                assert_eq!(count, 1, "failed validation rolls back any truncate");
            }
        }
        db.cleanup().await;
    });
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_copy_failure_rolls_back_replace_and_truncate() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let table = source(&dir, "invalid.csv", "id,note\n001,invalid\0text\n");
        for format in ["binary", "text"] {
            for mode in ["replace", "truncate"] {
                let name = format!("fail_{format}_{mode}");
                db.client.batch_execute(&format!("CREATE TABLE {}.{name} (id TEXT, note TEXT); INSERT INTO {}.{name} VALUES ('old', 'keep')", db.schema, db.schema)).await.unwrap();
                assert!(db
                    .import(
                        vec![table.clone()],
                        MergeOptions::default(),
                        db.request(&name, format, mode)
                    )
                    .await
                    .is_err());
                let rows = db
                    .client
                    .query(&format!("SELECT id, note FROM {}.{name}", db.schema), &[])
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get::<_, String>(0), "old");
                assert_eq!(rows[0].get::<_, String>(1), "keep");
            }
        }
        db.cleanup().await;
    });
}

#[test]
#[ignore = "requires an isolated PostgreSQL 17 service"]
fn postgres_integration_cancel_after_copy_batch_rolls_back_and_can_retry() {
    run(async {
        let db = Database::new().await;
        let dir = tempfile::tempdir().unwrap();
        let table = source(&dir, "input.csv", "id,note\n001,hello\n002,world\n");
        for format in ["binary", "text"] {
            db.client.batch_execute(&format!("CREATE TABLE {}.{format} (id TEXT, note TEXT); INSERT INTO {}.{format} VALUES ('old', 'keep')", db.schema, db.schema)).await.unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let observed = AtomicBool::new(false);
            let emit = |rows, _, _| {
                if rows > 0 {
                    observed.store(true, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                }
            };
            let result = import_database(
                vec![table.clone()],
                MergeOptions::default(),
                db.request(format, format, "truncate"),
                db.profile.clone(),
                Some(db.password.clone()),
                cancel.clone(),
                &emit,
            )
            .await
            .unwrap();
            assert!(
                observed.load(Ordering::Relaxed),
                "cancel after data reached COPY"
            );
            assert!(result.is_none());
            let rows = db
                .client
                .query(&format!("SELECT id FROM {}.{format}", db.schema), &[])
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get::<_, String>(0), "old");
            let result = db
                .import(
                    vec![table.clone()],
                    MergeOptions::default(),
                    db.request(format, format, "truncate"),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.rows, 2);
        }
        db.cleanup().await;
    });
}
