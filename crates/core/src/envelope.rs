//! The output envelope: `{"ok": true, "data": ...}` or
//! `{"ok": false, "error": {...}}`. Every error carries a stable code and the
//! next command to try.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub next: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Envelope {
    Ok { ok: bool, data: Value },
    Err { ok: bool, error: ErrorBody },
}

impl Envelope {
    #[must_use]
    pub const fn ok(data: Value) -> Self {
        Self::Ok { ok: true, data }
    }

    #[must_use]
    pub fn err(code: &str, message: &str, next: &str) -> Self {
        Self::Err {
            ok: false,
            error: ErrorBody {
                code: code.to_owned(),
                message: message.to_owned(),
                next: next.to_owned(),
            },
        }
    }

    #[must_use]
    pub const fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ok_shape() {
        let v = serde_json::to_value(Envelope::ok(json!({"x": 1}))).unwrap();
        assert_eq!(v, json!({"ok": true, "data": {"x": 1}}));
    }

    #[test]
    fn err_shape() {
        let v = serde_json::to_value(Envelope::err("E_NOPE", "nope", "assaymark status")).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["error"]["code"], json!("E_NOPE"));
        assert_eq!(v["error"]["next"], json!("assaymark status"));
    }
}
