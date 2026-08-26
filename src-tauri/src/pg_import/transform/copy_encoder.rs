use crate::merge::CellValue;

pub fn encode_copy_field(value: Option<&str>, output: &mut Vec<u8>) {
    let Some(value) = value else {
        output.extend_from_slice(br"\N");
        return;
    };
    let bytes = value.as_bytes();
    let mut start = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let escaped: Option<&[u8]> = match byte {
            92 => Some(&[92, 92]),
            9 => Some(&[92, b't']),
            10 => Some(&[92, b'n']),
            13 => Some(&[92, b'r']),
            _ => None,
        };
        if let Some(escaped) = escaped {
            output.extend_from_slice(&bytes[start..index]);
            output.extend_from_slice(escaped);
            start = index + 1;
        }
    }
    output.extend_from_slice(&bytes[start..]);
}

pub fn encode_copy_row_into(values: &[Option<String>], output: &mut Vec<u8>) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(b'\t');
        }
        encode_copy_field(value.as_deref(), output);
    }
    output.push(b'\n');
}

pub fn encode_copy_cells_row_into(values: &[CellValue], empty_as_null: bool, output: &mut Vec<u8>) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(b'\t');
        }
        let text = value.copy_text(empty_as_null);
        encode_copy_field(text.as_deref(), output);
    }
    output.push(b'\n');
}

#[must_use]
pub fn encode_copy_row(values: &[Option<String>]) -> Vec<u8> {
    let mut output = Vec::with_capacity(
        values
            .iter()
            .map(|value| value.as_ref().map_or(2, String::len) + 1)
            .sum(),
    );
    encode_copy_row_into(values, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_postgres_copy_text_specials() {
        let row = vec![
            Some("tab\there".into()),
            Some("line\nreturn\rslash\\".into()),
            Some(r"\N".into()),
            Some(String::new()),
            None,
            Some("中文".into()),
        ];
        assert_eq!(
            String::from_utf8(encode_copy_row(&row)).unwrap(),
            "tab\\there\tline\\nreturn\\rslash\\\\\t\\\\N\t\t\\N\t中文\n"
        );
    }

    #[test]
    fn appends_multiple_rows_to_one_copy_buffer() {
        let mut output = Vec::new();
        encode_copy_row_into(&[Some("1".into()), Some("甲".into())], &mut output);
        encode_copy_row_into(&[Some("2".into()), None], &mut output);
        assert_eq!(String::from_utf8(output).unwrap(), "1\t甲\n2\t\\N\n");
    }

    #[test]
    fn encodes_cell_values_with_the_same_copy_text_semantics() {
        let row = vec![
            CellValue::Text("001".into()),
            CellValue::Empty,
            CellValue::Integer(42),
            CellValue::Text(String::new()),
        ];
        let mut output = Vec::new();
        encode_copy_cells_row_into(&row, true, &mut output);
        assert_eq!(String::from_utf8(output).unwrap(), "001\t\\N\t42\t\\N\n");
    }

    #[test]
    fn distinguishes_null_empty_and_literal_null_words() {
        let row = vec![
            None,
            Some(String::new()),
            Some("NULL".into()),
            Some(r"\N".into()),
        ];
        assert_eq!(
            String::from_utf8(encode_copy_row(&row)).unwrap(),
            "\\N\t\tNULL\t\\\\N\n"
        );
    }
}
