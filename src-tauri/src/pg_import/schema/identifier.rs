use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const POSTGRES_IDENTIFIER_MAX_BYTES: usize = 63;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentifierMapping {
    pub original: String,
    pub database: String,
}

#[must_use]
pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[must_use]
pub fn map_identifiers(names: &[String]) -> Vec<IdentifierMapping> {
    let mut used = HashSet::with_capacity(names.len());
    names
        .iter()
        .map(|name| {
            let mut salt = 0u32;
            let database = loop {
                let candidate = if name.len() <= POSTGRES_IDENTIFIER_MAX_BYTES && salt == 0 {
                    name.clone()
                } else {
                    let digest = Sha256::digest(format!("{name}\0{salt}").as_bytes());
                    let hash = &hex::encode(digest)[..8];
                    let prefix_bytes = POSTGRES_IDENTIFIER_MAX_BYTES - 1 - hash.len();
                    format!("{}_{}", truncate_utf8(name, prefix_bytes), hash)
                };
                if used.insert(candidate.clone()) {
                    break candidate;
                }
                salt += 1;
            };
            IdentifierMapping {
                original: name.clone(),
                database,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_embedded_double_quotes() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_identifier("select"), "\"select\"");
    }

    #[test]
    fn truncates_by_utf8_bytes_and_avoids_collisions() {
        let common = "中".repeat(30);
        let names = vec![format!("{common}甲"), format!("{common}乙")];
        let mapped = map_identifiers(&names);
        assert!(mapped.iter().all(|m| m.database.len() <= 63));
        assert_ne!(mapped[0].database, mapped[1].database);
        assert!(mapped[0]
            .database
            .is_char_boundary(mapped[0].database.len()));
    }
}
