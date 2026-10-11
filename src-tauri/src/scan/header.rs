use crate::model::{header_key, make_default_mappings, normalize_headers, SourceKind, SourceTable};
use std::collections::HashSet;
use std::path::Path;
pub(super) const AUTO_HEADER_ROWS: usize = 20;
pub(super) fn source_table_from_headers(
    path: &Path,
    sheet_name: String,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
    headers: Vec<String>,
    estimated_rows: u64,
) -> SourceTable {
    SourceTable {
        path: path.to_owned(),
        sheet_name,
        kind: SourceKind::Workbook,
        header_row: header_row.max(1),
        header_rows: header_rows.clamp(1, 3),
        suggested_header_row: suggested_header_row.max(1),
        mappings: make_default_mappings(&headers),
        headers,
        estimated_rows,
        enabled: true,
    }
}

pub(super) fn combine_header_rows(rows: &[Vec<String>]) -> Vec<String> {
    combine_header_rows_with_width(rows, rows.iter().map(Vec::len).max().unwrap_or(0))
}

pub(super) fn combine_header_rows_with_width(rows: &[Vec<String>], width: usize) -> Vec<String> {
    normalize_headers((0..width).map(|column| {
        let mut seen = Vec::new();
        for row in rows {
            let value = row
                .get(column)
                .map(|value| value.trim())
                .unwrap_or_default();
            if !value.is_empty()
                && !seen
                    .iter()
                    .any(|existing: &String| header_key(existing) == header_key(value))
            {
                seen.push(value.to_owned());
            }
        }
        seen.join(" / ")
    }))
}

pub fn recommend_header<I>(rows: I) -> usize
where
    I: IntoIterator<Item = Vec<String>>,
{
    let rows = rows.into_iter().take(AUTO_HEADER_ROWS).collect::<Vec<_>>();
    if rows.is_empty() {
        return 0;
    }
    let max_width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let non_empty = row.iter().filter(|cell| !cell.trim().is_empty()).count();
            let unique = row
                .iter()
                .filter(|cell| !cell.trim().is_empty())
                .map(|cell| cell.trim().to_lowercase())
                .collect::<HashSet<_>>()
                .len();
            let text = row
                .iter()
                .filter(|cell| cell.chars().any(char::is_alphabetic))
                .count();
            let next_same = rows
                .get(index + 1)
                .is_some_and(|next| next.len() == row.len() && row.len() > 1);
            let score = non_empty as i64 * 20 / max_width as i64
                + unique as i64 * 4
                + text as i64 * 3
                + i64::from(next_same) * 8
                - index as i64;
            (score, std::cmp::Reverse(index))
        })
        .max()
        .map(|(_, std::cmp::Reverse(index))| index)
        .unwrap_or(0)
}
