use crate::merge::CellValue;
use crate::pg_import::{AppError, Result};

const SIGNATURE: &[u8; 11] = b"PGCOPY\n\xFF\r\n\0";

pub fn encode_binary_header(output: &mut Vec<u8>) {
    output.extend_from_slice(SIGNATURE);
    output.extend_from_slice(&0i32.to_be_bytes());
    output.extend_from_slice(&0i32.to_be_bytes());
}

pub fn encode_binary_row(values: &[Option<String>], output: &mut Vec<u8>) -> Result<()> {
    let field_count = i16::try_from(values.len()).map_err(|_| {
        AppError::Config(format!(
            "二进制 COPY 单行字段数超过 PostgreSQL 限制：{}",
            values.len()
        ))
    })?;
    output.extend_from_slice(&field_count.to_be_bytes());
    for value in values {
        match value {
            None => output.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(value) => {
                let length = i32::try_from(value.len()).map_err(|_| {
                    AppError::Config("单个字段超过 PostgreSQL 二进制 COPY 的 2 GiB 限制".into())
                })?;
                output.extend_from_slice(&length.to_be_bytes());
                output.extend_from_slice(value.as_bytes());
            }
        }
    }
    Ok(())
}

pub fn encode_binary_cells_row(
    values: &[CellValue],
    empty_as_null: bool,
    output: &mut Vec<u8>,
) -> Result<()> {
    let field_count = i16::try_from(values.len()).map_err(|_| {
        AppError::Config(format!(
            "二进制 COPY 单行字段数超过 PostgreSQL 限制：{}",
            values.len()
        ))
    })?;
    output.extend_from_slice(&field_count.to_be_bytes());
    for value in values {
        match value.copy_text(empty_as_null) {
            None => output.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(value) => {
                let length = i32::try_from(value.len()).map_err(|_| {
                    AppError::Config("单个字段超过 PostgreSQL 二进制 COPY 的 2 GiB 限制".into())
                })?;
                output.extend_from_slice(&length.to_be_bytes());
                output.extend_from_slice(value.as_bytes());
            }
        }
    }
    Ok(())
}

pub fn encode_binary_trailer(output: &mut Vec<u8>) {
    output.extend_from_slice(&(-1i16).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_postgres_binary_copy_stream() {
        let mut output = Vec::new();
        encode_binary_header(&mut output);
        encode_binary_row(
            &[
                Some("A".into()),
                None,
                Some(String::new()),
                Some("中文".into()),
            ],
            &mut output,
        )
        .unwrap();
        encode_binary_trailer(&mut output);

        assert_eq!(&output[..11], SIGNATURE);
        assert_eq!(&output[11..19], &[0; 8]);
        assert_eq!(&output[19..21], &4i16.to_be_bytes());
        assert_eq!(&output[21..25], &1i32.to_be_bytes());
        assert_eq!(output[25], b'A');
        assert_eq!(&output[26..30], &(-1i32).to_be_bytes());
        assert_eq!(&output[30..34], &0i32.to_be_bytes());
        assert_eq!(&output[34..38], &6i32.to_be_bytes());
        assert_eq!(&output[38..44], "中文".as_bytes());
        assert_eq!(&output[44..], &(-1i16).to_be_bytes());
    }

    #[test]
    fn cell_value_encoder_matches_owned_string_encoder() {
        let cells = vec![
            CellValue::Text("A".into()),
            CellValue::Empty,
            CellValue::Integer(42),
        ];
        let owned = vec![Some("A".into()), None, Some("42".into())];
        let mut direct_output = Vec::new();
        let mut owned_output = Vec::new();
        encode_binary_cells_row(&cells, true, &mut direct_output).unwrap();
        encode_binary_row(&owned, &mut owned_output).unwrap();
        assert_eq!(direct_output, owned_output);
    }
}
