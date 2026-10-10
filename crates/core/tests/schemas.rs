//! The schemas are the contract: each must be a valid schema, each example
//! must validate against its schema, and the run example must cite the
//! bundle example by its real content hash.

use assaymark_core::{content_id, IdKind};
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(rel: &str) -> Value {
    let p = root().join(rel);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn check(schema: &str, example: &str) {
    let schema = load(schema);
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let doc = load(example);
    let errors: Vec<String> = validator.iter_errors(&doc).map(|e| format!("{e} at {}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{example}: {errors:#?}");
}

fn rejects(schema: &str, doc: &Value) {
    let validator = jsonschema::validator_for(&load(schema)).expect("schema compiles");
    assert!(!validator.is_valid(doc), "expected rejection: {doc}");
}

#[test]
fn bundle_example_validates() {
    check("schemas/bundle.schema.json", "schemas/examples/bundle.video-clock.json");
}

#[test]
fn composer_example_validates() {
    check("schemas/composer.schema.json", "schemas/examples/composer.wmc-exact.json");
}

#[test]
fn profile_example_validates() {
    check("schemas/profile.schema.json", "schemas/examples/profile.gate-strict.json");
}

#[test]
fn run_example_validates() {
    check("schemas/run.schema.json", "schemas/examples/run.video-clock.json");
}

#[test]
fn run_example_cites_the_real_bundle_hash() {
    let bundle = load("schemas/examples/bundle.video-clock.json");
    let run = load("schemas/examples/run.video-clock.json");
    assert_eq!(run["bundle"], Value::String(content_id(IdKind::Bundle, &bundle)));
}

#[test]
fn run_example_id_is_the_hash_of_its_hashed_fields() {
    let mut run = load("schemas/examples/run.video-clock.json");
    let id = run["id"].clone();
    let obj = run.as_object_mut().unwrap();
    obj.remove("id");
    obj.remove("created_at");
    if let Some(steps) = obj.get_mut("steps").and_then(Value::as_array_mut) {
        for s in steps {
            s.as_object_mut().unwrap().remove("elapsed_ms");
        }
    }
    assert_eq!(id, Value::String(content_id(IdKind::Run, &run)));
}

#[test]
fn bundle_schema_rejects_domain_fields() {
    let mut b = load("schemas/examples/bundle.video-clock.json");
    b["claims"][0]["lr_plus"] = Value::from(19);
    rejects("schemas/bundle.schema.json", &b);
}

#[test]
fn bundle_schema_rejects_unweighted_undercut_without_strength() {
    let mut b = load("schemas/examples/bundle.video-clock.json");
    b["links"][1].as_object_mut().unwrap().remove("strength");
    rejects("schemas/bundle.schema.json", &b);
}

#[test]
fn run_schema_rejects_malformed_ids() {
    let mut r = load("schemas/examples/run.video-clock.json");
    r["id"] = Value::from("run:s256:nothex");
    rejects("schemas/run.schema.json", &r);
}
