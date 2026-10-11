//! The binary end to end, with a fake `condorcet-adapter-fx` executable
//! that serves fixture bundles by `staged` id. The gate contract is the
//! one kd-share's promotion gate parses: `{"ok":true,"data":{"verdict":
//! "pass"|"hold"|"refuse","reasons":[...],"run":"run:s256:.."}}`, or a
//! typed error envelope, always with exit status 0.

use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Env {
    dir: PathBuf,
}

fn bundle(claims: &Value, links: &Value, constraints: &Value) -> Value {
    json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "fx", "version": "1", "revision": "git:abc"},
           "claims": claims, "exhibits": [
               {"id": "fx:ex1", "reading": {"claim": "fx:d/good"}},
               {"id": "fx:ex2", "reading": {"claim": "fx:d/doubted"}}],
           "links": links, "constraints": constraints})
}

impl Env {
    fn new(name: &str) -> Self {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("condorcet-cli-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let claims = json!([
            {"id": "fx:d/good", "text": "well supported", "prior": 0.5},
            {"id": "fx:d/doubted", "text": "contradicted", "prior": 0.5},
            {"id": "fx:d/rival", "text": "excludes good", "prior": 0.5}
        ]);
        let support = json!({"id": "link:fx:ex1>fx:d/good", "kind": "support", "from": "fx:ex1", "to": "fx:d/good", "weight": {"reliability": 0.9}});
        let contra = json!({"id": "link:fx:ex2>fx:d/doubted", "kind": "contradict", "from": "fx:ex2", "to": "fx:d/doubted", "weight": {"reliability": 0.9}});
        let fixtures = [
            ("good", bundle(&claims, &json!([support]), &json!([]))),
            ("doubted", bundle(&claims, &json!([support, contra]), &json!([]))),
            ("rival", bundle(&claims, &json!([support]), &json!([{"id": "k", "kind": "excludes", "claims": ["fx:d/good", "fx:d/rival"], "mode": "hard"}]))),
        ];
        for (k, v) in &fixtures {
            std::fs::write(dir.join(format!("{k}.json")), v.to_string()).unwrap();
        }
        let script = format!(
            r#"#!/bin/sh
D={d}
case "$1" in
  capabilities) echo '{{"ok":true,"data":{{"id":"fx","version":"1","protocol":"condorcet/adapter/v1","load":true,"write_back":true,"profile":true}}}}' ;;
  load) s=$(printf '%s' "$3" | sed -n 's/.*"staged":"\([^"]*\)".*/\1/p'); f="$D/$s.json"
        if [ -f "$f" ]; then printf '{{"ok":true,"data":'; cat "$f"; printf '}}'; else echo '{{"ok":false,"error":{{"code":"E_STAGED_NOT_FOUND","message":"no such fact","next":"kd status"}}}}'; fi ;;
  profile) if [ "$2" = fre-default ]; then cat "$D/profile.json"; else echo '{{"ok":false,"error":{{"code":"E_PROFILE_NOT_FOUND","message":"none","next":"x"}}}}'; fi ;;
  write-back) printf '{{"ok":true,"data":{{"mode":"%s","revision":"git:abc","stdin_bytes":%s}}}}' "$([ "$2" = --commit ] && echo commit || echo preview)" "$(wc -c | tr -d ' ')" ;;
esac
"#,
            d = dir.display()
        );
        let bin = dir.join("bin/condorcet-adapter-fx");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = json!({"ok": true, "data": {"name": "fre-default", "version": "1",
            "steps": [{"composer": "sat-core@1", "on_fail": "stop"}, {"composer": "wmc-exact@1"}],
            "gates": [{"id": "consistent", "rule": "sat-core.satisfiable == true", "on_fail": "refuse"},
                      {"id": "floor", "rule": "wmc-exact.conditional(load_bearing) >= 0.95"}]}});
        std::fs::write(dir.join("profile.json"), profile.to_string()).unwrap();
        Self { dir }
    }

    fn run(&self, args: &[&str]) -> (Value, i32) {
        let path = format!("{}:{}", self.dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let out = Command::new(env!("CARGO_BIN_EXE_condorcet"))
            .args(args)
            .env("PATH", path)
            .env("CONDORCET_HOME", self.dir.join("store"))
            .current_dir(&self.dir)
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (v, out.status.code().unwrap_or(-1))
    }

    fn gate(&self, staged: &str, profile: &str) -> (Value, i32) {
        self.run(&["gate", "--adapter", "fx", "--profile", profile, "--staged", staged])
    }
}

fn verdict(v: &Value) -> &str {
    v["data"]["verdict"].as_str().unwrap_or("?")
}

#[test]
fn gate_reports_pass_hold_refuse_in_the_kd_contract() {
    let e = Env::new("gate");
    let (v, code) = e.gate("good", "gate-default");
    assert_eq!((verdict(&v), code), ("pass", 0), "{v}");
    assert!(v["data"]["run"].as_str().unwrap().starts_with("run:s256:"));
    assert!(v["data"]["reasons"].as_array().unwrap().is_empty());
    let (v, _) = e.gate("doubted", "gate-default");
    assert_eq!(verdict(&v), "hold", "{v}");
    assert!(v["data"]["reasons"].to_string().contains("not-defeated failed"));
    let (v, _) = e.gate("rival", "gate-default");
    assert_eq!(verdict(&v), "refuse", "{v}");
    assert!(v["data"]["reasons"].to_string().contains("unsat core"));
}

#[test]
fn gate_asks_the_adapter_for_unknown_profiles() {
    let e = Env::new("profile");
    // 0.9 posterior is under the adapter profile's 0.95 floor.
    let (v, _) = e.gate("good", "fre-default");
    assert_eq!(verdict(&v), "hold", "{v}");
    assert_eq!(v["data"]["profile"], "fre-default@1");
}

#[test]
fn gate_bad_input_is_a_typed_error_with_exit_zero() {
    let e = Env::new("errors");
    for (args, code) in [
        (vec!["gate", "--adapter", "fx", "--profile", "gate-default"], "E_USAGE"),
        (vec!["gate", "--adapter", "fx", "--profile", "gate-default", "--staged", "missing"], "E_STAGED_NOT_FOUND"),
        (vec!["gate", "--adapter", "nope", "--profile", "gate-default", "--staged", "good"], "E_ADAPTER_NOT_FOUND"),
        (vec!["gate", "--adapter", "fx", "--profile", "unknown", "--staged", "good"], "E_PROFILE_NOT_FOUND"),
        (vec!["gate", "--adapter", "fx", "--profile"], "E_USAGE"),
    ] {
        let (v, exit) = e.run(&args);
        assert_eq!((v["ok"].as_bool(), v["error"]["code"].as_str(), exit), (Some(false), Some(code), 0), "{args:?}: {v}");
        assert!(v["error"]["next"].as_str().is_some_and(|n| !n.is_empty()));
    }
    let (v, exit) = e.run(&["frobnicate"]);
    assert_eq!((v["error"]["code"].as_str(), exit), (Some("E_UNKNOWN_COMMAND"), 2));
}

#[test]
fn compose_previews_then_commits_and_rerun_is_memoised() {
    let e = Env::new("compose");
    let scope = r#"{"staged":"good"}"#;
    let (v, _) = e.run(&["compose", "--adapter", "fx", "--scope", scope, "--profile", "gate-default"]);
    assert_eq!(v["data"]["write_back"]["mode"], "preview", "{v}");
    assert_eq!(v["data"]["memoised"], false);
    let run_id = v["data"]["run"]["id"].as_str().unwrap().to_owned();
    let (v, _) = e.run(&["compose", "--adapter", "fx", "--scope", scope, "--profile", "gate-default", "--commit"]);
    assert_eq!(v["error"]["code"], "E_USAGE");
    let (v, _) = e.run(&["compose", "--adapter", "fx", "--scope", scope, "--profile", "gate-default", "--commit", "--expected-revision", "git:abc"]);
    assert_eq!((v["data"]["write_back"]["mode"].as_str(), v["data"]["memoised"].as_bool()), (Some("commit"), Some(true)));
    assert_eq!(v["data"]["run"]["id"], run_id.as_str());
    let (v, _) = e.run(&["explain", &run_id]);
    assert_eq!(v["data"]["verdict"], "pass");
    assert_eq!(v["data"]["claims"]["fx:d/good"], "well supported");
    let (v, _) = e.run(&["resolve", &run_id]);
    assert_eq!(v["data"]["kind"], "run");
    let log = std::fs::read_to_string(e.dir.join("store/runs.ndjson")).unwrap();
    assert_eq!(log.lines().count(), 1);
}

#[test]
fn single_composer_verbs_and_validation() {
    let e = Env::new("verbs");
    let f = e.dir.join("doubted.json");
    let f = f.to_str().unwrap();
    let (v, _) = e.run(&["argue", "--bundle", f, "--semantics", "preferred"]);
    assert_eq!(v["data"]["outputs"]["label"]["fx:d/doubted"], "out", "{v}");
    let (v, _) = e.run(&["core", "--bundle", f]);
    assert_eq!(v["data"]["outputs"]["satisfiable"], true);
    let (v, _) = e.run(&["joint", "--bundle", f, "fx:d/good", "fx:d/doubted"]);
    let p = v["data"]["joint"].as_f64().unwrap();
    assert!((p - 0.9 / 1.1 * (0.1 / 1.1)).abs() < 1e-12, "{v}");
    let (v, _) = e.run(&["jury", "--bundle", f]);
    assert!((v["data"]["outputs"]["majority_correct"]["fx:d/good"].as_f64().unwrap() - 0.9).abs() < 1e-12);
    let (v, _) = e.run(&["bundle", "--bundle", f, "--validate"]);
    assert_eq!(v["data"]["valid"], true, "{v}");
    let (v, _) = e.run(&["profile", "validate", &format!("{}/../../profiles/gate-strict.json", env!("CARGO_MANIFEST_DIR"))]);
    assert_eq!(v["data"]["valid"], true, "{v}");
    let (v, _) = e.run(&["composer", "list"]);
    assert_eq!(v["data"].as_array().unwrap().len(), 4);
    let (v, _) = e.run(&["status"]);
    assert!(v["data"]["adapters"].as_array().unwrap().contains(&json!("fx")));
    let _ = Path::new(f);
}
