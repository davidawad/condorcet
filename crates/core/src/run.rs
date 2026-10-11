//! The composition run record (schema `condorcet/run/v1`): append-only,
//! content-addressed.

use crate::id::{content_id, IdKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const RUN_SCHEMA: &str = "condorcet/run/v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Evidence {
    #[must_use]
    pub fn new(kind: &str, ids: Vec<String>, data: Option<Value>) -> Self {
        Self { kind: kind.to_owned(), ids, data }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepRecord {
    pub composer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// ok | failed | skipped | budget_exceeded
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<StepError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateRecord {
    pub id: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameVersion {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    pub schema: String,
    pub id: String,
    pub bundle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<crate::bundle::AdapterProvenance>,
    pub profile: NameVersion,
    pub composers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub memo_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub steps: Vec<StepRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateRecord>,
    /// pass | fail | inconclusive
    pub verdict: String,
}

/// `s256:<hex>` of the canonical JSON of `doc`.
#[must_use]
pub fn hash(doc: &Value) -> String {
    format!("s256:{:x}", Sha256::digest(crate::id::canonical_json(doc).as_bytes()))
}

/// The memo key: identical (bundle, profile, composer versions, seed,
/// query) gives an identical key, so a re-run is a lookup.
#[must_use]
pub fn memo_key(bundle_id: &str, profile: &Value, composers: &[String], seed: Option<u64>, query: &Value) -> String {
    hash(&serde_json::json!({
        "bundle": bundle_id, "profile": profile, "composers": composers, "seed": seed, "query": query
    }))
}

impl RunRecord {
    /// The record as JSON with the fields excluded from the content hash
    /// removed: `id`, `created_at`, each step's `elapsed_ms`.
    #[must_use]
    pub fn hashed_view(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.remove("id");
            o.remove("created_at");
            if let Some(steps) = o.get_mut("steps").and_then(Value::as_array_mut) {
                for s in steps.iter_mut().filter_map(Value::as_object_mut) {
                    s.remove("elapsed_ms");
                }
            }
        }
        v
    }

    /// Set `id` to the content hash of the record.
    pub fn seal(&mut self) {
        self.id = content_id(IdKind::Run, &self.hashed_view());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_round_trips_and_seals_to_its_id() {
        let v: Value =
            serde_json::from_str(include_str!("../../../schemas/examples/run.video-clock.json")).unwrap();
        let mut r: RunRecord = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&r).unwrap(), v);
        let id = r.id.clone();
        r.created_at = Some("2030-01-01T00:00:00Z".into());
        r.seal();
        assert_eq!(r.id, id);
    }
}
