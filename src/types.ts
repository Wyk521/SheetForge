// 与 Rust 侧 serde 序列化对应的类型（字段名保持 snake_case）

export type MergeMode = "Union" | "Intersection" | "Manual" | "Consolidate" | "Join";
export type SourceKind = { Csv: { delimiter: number } } | "Workbook";
export type TransformOp = "None" | "Trim" | "Uppercase" | "Lowercase";
export type AggregateOp = "First" | "Sum" | "UniqueJoin" | "TextJoin";
export type JoinKind = "Left" | "Inner" | "Full";
export type IssueLevel = "Info" | "Warning" | "Error";

export interface ColumnMapping {
  source_index: number;
  source_name: string;
  target_name: string;
  enabled: boolean;
  transform: TransformOp;
  aggregate: AggregateOp;
}

export interface SourceTable {
  path: string;
  sheet_name: string;
  kind: SourceKind;
  header_row: number;
  header_rows: number;
  suggested_header_row: number;
  headers: string[];
  estimated_rows: number;
  enabled: boolean;
  mappings: ColumnMapping[];
}

export interface MergeOptions {
  mode: MergeMode;
  include_source_file: boolean;
  include_source_sheet: boolean;
  output_order: string[];
  deduplicate: boolean;
  key_columns: string[];
  join_kind: JoinKind;
  text_join_separator: string;
  filter_column: string;
  filter_text: string;
  filter_exclude: boolean;
}

export interface PreviewTable {
  headers: string[];
  rows: string[][];
}

export interface CheckIssue {
  level: IssueLevel;
  title: string;
  detail: string;
}

export interface MergeScheme {
  format_version: number;
  name: string;
  tables: SourceTable[];
  options: MergeOptions;
}

export interface AppSettings {
  output_directory: string;
  recent_folders: string[];
  recent_schemes: string[];
  window_maximized: boolean;
  check_updates: boolean;
}

// 事件负载
export interface ScanProgress {
  done: number;
  total: number;
  name: string;
}

export interface ScanFinished {
  tables: SourceTable[];
  warnings: string[];
}

export interface TableReloaded {
  index: number;
  table: SourceTable;
}

export interface TablesReloaded {
  tables: TableReloaded[];
  failures: number;
}

export interface MergeProgress {
  current: number;
  total: number;
  label: string;
}

export interface MergeFinished {
  output: string;
  rows: number;
  sheets: number;
}

export interface PreflightDone {
  issues: CheckIssue[];
  continues_merge: boolean;
}

export interface UpdateResult {
  version: string;
  url: string;
  newer: boolean;
}

export const defaultOptions = (): MergeOptions => ({
  mode: "Union",
  include_source_file: false,
  include_source_sheet: false,
  output_order: [],
  deduplicate: false,
  key_columns: [],
  join_kind: "Left",
  text_join_separator: "；",
  filter_column: "",
  filter_text: "",
  filter_exclude: false,
});
