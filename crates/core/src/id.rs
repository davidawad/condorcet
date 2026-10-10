//! Content-addressed ids. Every id names its kind: `bundle:s256:<hex>`,
//! `run:s256:<hex>`.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The kinds of id that are hashes of a canonical document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Bundle,
    Run,
}

impl IdKind {
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Bundle => "bundle",
            Self::Run => "run",
        }
    }
}

/// Canonical JSON: object keys sorted (serde_json's default map is ordered),
/// no whitespace. Equal documents give equal bytes.
#[must_use]
pub fn canonical_json(doc: &Value) -> String {
    doc.to_string()
}

/// `<kind>:s256:<hex sha256 of the canonical JSON>`.
#[must_use]
pub fn content_id(kind: IdKind, doc: &Value) -> String {
    let digest = Sha256::digest(canonical_json(doc).as_bytes());
    format!("{}:s256:{digest:x}", kind.prefix())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_the_id() {
        let a = json!({"a": 1, "b": [1, 2]});
        let b = json!({"b": [1, 2], "a": 1});
        assert_eq!(content_id(IdKind::Bundle, &a), content_id(IdKind::Bundle, &b));
    }

    #[test]
    fn id_names_its_kind_and_hash() {
        let id = content_id(IdKind::Run, &json!({}));
        assert!(id.starts_with("run:s256:"));
        assert_eq!(id.len(), "run:s256:".len() + 64);
    }
}
