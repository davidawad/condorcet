//! The commands.

use crate::args::Args;
use crate::registry;
use crate::Fail;
use condorcet_adapter::{discover, ExecAdapter, FactAdapter, FileAdapter, WriteMode};
use condorcet_composer::{memo_key, run_profile, Profile, Query};
use condorcet_core::{content_id, Bundle, IdKind, RunRecord};
use condorcet_store::{default_root, Store};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Duration;

const HELP: &str = "condorcet status | composer list|show <name>|validate <file> | profile list|show <name> | adapters \
| bundle (--bundle F | --adapter A --scope J) | compose (--bundle F | --adapter A --scope J) --profile P [--set name=a,b] [--focus C] [--commit --expected-revision R] [--write-estimates] \
| joint --bundle F <claim>... | core --bundle F [--max-cores N] | argue --bundle F [--semantics grounded|preferred|stable] | jury --bundle F \
| gate --adapter A --profile P --staged ID [--proposed PATH] | explain <run id> | resolve <id>";

type R = Result<Value, Fail>;

pub fn dispatch(a: &Args) -> R {
    let pos: Vec<&str> = a.positional.iter().map(String::as_str).collect();
    match pos.as_slice() {
        [] | ["status"] => Ok(status()),
        ["help"] => Ok(json!({"usage": HELP})),
        ["composer", "list"] => Ok(json!(registry::COMPOSER_SPECS.iter().map(|(n, s)| {
            let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
            json!({"id": format!("{n}@{}", v["version"].as_str().unwrap_or("1")), "family": v["family"], "title": v["title"], "exactness": v["exactness"]})
        }).collect::<Vec<_>>())),
        ["composer", "show", name] => registry::spec(name)
            .ok_or_else(|| Fail::new("E_COMPOSER_UNKNOWN", format!("no composer {name}"), "condorcet composer list")),
        ["composer", "validate", file] => validate_file("composer", file),
        ["profile", "list"] => Ok(json!(registry::PROFILES.iter().map(|(n, _)| *n).collect::<Vec<_>>())),
        ["profile", "show", name] => Ok(resolve_profile(name, a.get("adapter"))?.to_value()),
        ["profile", "validate", file] => validate_file("profile", file),
        ["adapters"] => Ok(json!(discover().into_iter().map(|(id, p)| json!({"id": id, "bin": p})).collect::<Vec<_>>())),
        ["bundle"] => bundle_cmd(a),
        ["compose"] => compose(a),
        ["joint", claims @ ..] => joint(a, claims),
        ["core"] => single(a, "sat-core@1", json!({"max_cores": a.get("max-cores").and_then(|x| x.parse::<u64>().ok()).unwrap_or(1)})),
        ["argue"] => single(a, "dung@1", json!({"semantics": a.get("semantics").unwrap_or("grounded")})),
        ["jury"] => single(a, "condorcet-jury@1", json!({})),
        ["gate"] => gate(a),
        ["explain", id] => explain(id),
        ["resolve", id] => resolve(id),
        _ => Err(Fail::new("E_UNKNOWN_COMMAND", format!("unknown command: {}", pos.join(" ")), "condorcet help")),
    }
}

fn status() -> Value {
    json!({
        "name": "condorcet",
        "version": env!("CARGO_PKG_VERSION"),
        "composers": registry::composer_ids(),
        "profiles": registry::PROFILES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "adapters": discover().into_iter().map(|(id, _)| id).chain(["file".to_owned()]).collect::<Vec<_>>(),
        "store": default_root(),
        "protocol": condorcet_adapter::PROTOCOL,
    })
}

fn read_json(path: &str) -> R {
    let text = std::fs::read_to_string(path).map_err(|e| Fail::new("E_FILE_UNREADABLE", format!("{path}: {e}"), "ls <file>"))?;
    serde_json::from_str(&text).map_err(|e| Fail::new("E_JSON_INVALID", format!("{path}: {e}"), "jq . <file>"))
}

fn schema_errors(kind: &str, doc: &Value) -> Vec<String> {
    let Some(schema) = registry::schema(kind) else { return vec![format!("no schema {kind}")] };
    match jsonschema::validator_for(&schema) {
        Ok(v) => v.iter_errors(doc).map(|e| format!("{e} at {}", e.instance_path())).collect(),
        Err(e) => vec![e.to_string()],
    }
}

fn validate_file(kind: &str, file: &str) -> R {
    let doc = read_json(file)?;
    let errors = schema_errors(kind, &doc);
    if !errors.is_empty() {
        return Err(Fail::new("E_SCHEMA_INVALID", errors.join("; "), &format!("condorcet {kind} show <name>")));
    }
    if kind == "profile" {
        Profile::from_json(doc).map_err(|(c, m)| Fail::new(&c, m, "condorcet profile list"))?;
    }
    Ok(json!({"valid": true, "kind": kind}))
}

fn timeout() -> Duration {
    Duration::from_secs(std::env::var("CONDORCET_ADAPTER_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(8))
}

fn adapter(id: &str) -> Result<Box<dyn FactAdapter>, Fail> {
    if id == "file" {
        return Ok(Box::new(FileAdapter));
    }
    Ok(Box::new(ExecAdapter::find(id, timeout())?))
}

fn parse_scope(a: &Args) -> R {
    match a.get("scope") {
        None => Ok(json!({})),
        Some(s) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| Fail::new("E_SCOPE_INVALID", "--scope must be a JSON object", "condorcet help")),
    }
}

/// The bundle a command works on: `--bundle <file | bundle:s256:..>`, or
/// `--adapter A --scope J`. Returns the bundle, the adapter (if any) and the scope.
fn source(a: &Args) -> Result<(Bundle, Option<Box<dyn FactAdapter>>, Value), Fail> {
    if let Some(b) = a.get("bundle") {
        if b.starts_with("bundle:") {
            return Ok((Store::open(default_root()).get_bundle(b)?, None, json!({})));
        }
        let scope = json!({"path": b});
        return Ok((FileAdapter.load(&scope)?, None, scope));
    }
    let id = a.need("adapter", "condorcet compose --bundle <file> | --adapter <id> --scope <json>")?;
    let ad = adapter(id)?;
    let scope = parse_scope(a)?;
    let b = ad.load(&scope)?;
    Ok((b, Some(ad), scope))
}

fn bundle_id(b: &Bundle) -> String {
    content_id(IdKind::Bundle, &serde_json::to_value(b).unwrap_or(Value::Null))
}

fn bundle_cmd(a: &Args) -> R {
    let (b, _, _) = source(a)?;
    let id = Store::open(default_root()).put_bundle(&b).unwrap_or_else(|_| bundle_id(&b));
    let errors = schema_errors("bundle", &serde_json::to_value(&b).unwrap_or(Value::Null));
    if a.has("validate") {
        return Ok(json!({"id": id, "valid": errors.is_empty(), "errors": errors}));
    }
    Ok(json!({"id": id, "bundle": b}))
}

fn resolve_profile(name: &str, adapter_id: Option<&str>) -> Result<Profile, Fail> {
    let parse = |v: Value| Profile::from_json(v).map_err(|(c, m)| Fail::new(&c, m, "condorcet profile validate <file>"));
    let p = Path::new(name);
    if p.extension().is_some_and(|e| e == "json") || name.contains('/') {
        return parse(read_json(name)?);
    }
    if let Some(dirs) = std::env::var_os("CONDORCET_PROFILES") {
        for d in std::env::split_paths(&dirs) {
            let f = d.join(format!("{name}.json"));
            if f.is_file() {
                return parse(read_json(&f.to_string_lossy())?);
            }
        }
    }
    if let Some(p) = registry::builtin_profile(name) {
        return Ok(p);
    }
    if let Some(id) = adapter_id.filter(|&i| i != "file") {
        return parse(adapter(id)?.profile(name)?);
    }
    Err(Fail::new("E_PROFILE_NOT_FOUND", format!("no profile {name}"), "condorcet profile list"))
}

fn query(a: &Args, b: &Bundle, focus: Option<String>) -> Result<Query, Fail> {
    let mut q = Query::default_for(b, focus.or_else(|| a.get("focus").map(str::to_owned)));
    for s in a.all("set") {
        let (name, ids) = s.split_once('=').ok_or_else(|| Fail::new("E_USAGE", format!("--set {s}: expected name=a,b"), "condorcet help"))?;
        q.sets.insert(name.to_owned(), ids.split(',').filter(|x| !x.is_empty()).map(str::to_owned).collect());
    }
    let ix = b.claim_index();
    let unknown: Vec<&String> = q.sets.values().flatten().chain(q.focus.iter()).filter(|c| !ix.contains_key(c.as_str())).collect();
    if let Some(c) = unknown.first() {
        return Err(Fail::new("E_QUERY_INVALID", format!("{c} is not a claim in the bundle"), "condorcet bundle --bundle <file>"));
    }
    Ok(q)
}

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Run (or recall) `profile` over `b`; store bundle and run. Returns the
/// record, whether it was memoised, and a store warning if the store failed.
fn run(b: &Bundle, profile: &Profile, q: &Query) -> (RunRecord, bool, Option<String>) {
    let store = Store::open(default_root());
    let key = memo_key(&bundle_id(b), profile, q);
    if let Some(r) = store.find_memo(&key) {
        return (r, true, None);
    }
    let mut r = run_profile(b, profile, q, &registry::composer);
    r.created_at = Some(now_rfc3339());
    let warn = store.put_bundle(b).and_then(|_| store.append_run(&r)).err().map(|e| e.to_string());
    (r, false, warn)
}

fn compose(a: &Args) -> R {
    let (b, ad, scope) = source(a)?;
    let profile = resolve_profile(a.need("profile", "condorcet profile list")?, a.get("adapter"))?;
    let q = query(a, &b, None)?;
    let (r, memoised, warn) = run(&b, &profile, &q);
    let mut data = json!({"run": r, "memoised": memoised});
    if let Some(w) = warn {
        data["warning"] = json!(w);
    }
    let Some(ad) = ad else { return Ok(data) };
    if !ad.capabilities().is_ok_and(|c| c.write_back) {
        data["write_back"] = json!({"mode": "unsupported"});
        return Ok(data);
    }
    let mode = if a.has("commit") {
        WriteMode::Commit {
            expected_revision: a.need("expected-revision", "condorcet compose ... --commit --expected-revision <rev from the preview>")?.to_owned(),
        }
    } else {
        WriteMode::Preview
    };
    data["write_back"] = ad.write_back(&r, &scope, a.has("write-estimates"), &mode)?;
    Ok(data)
}

fn single(a: &Args, composer: &str, params: Value) -> R {
    let (b, _, _) = source(a)?;
    let q = query(a, &b, None)?;
    let c = registry::composer(composer).ok_or_else(|| Fail::new("E_COMPOSER_UNKNOWN", composer, "condorcet composer list"))?;
    let budget = c.spec().default_budget().started();
    let out = c.compose(&b, &params, &q, &budget).map_err(|e| Fail::new(&e.code, e.message, &format!("condorcet composer show {composer}")))?;
    Ok(json!({"composer": composer, "bundle": bundle_id(&b), "outputs": out.outputs, "evidence": out.evidence}))
}

fn joint(a: &Args, claims: &[&str]) -> R {
    let (b, _, _) = source(a)?;
    if claims.is_empty() {
        return Err(Fail::new("E_USAGE", "joint needs at least one claim id", "condorcet joint --bundle <file> <claim>..."));
    }
    let mut q = query(a, &b, None)?;
    q.sets.insert("query".into(), claims.iter().map(|&c| c.to_owned()).collect());
    let ix = b.claim_index();
    if let Some(c) = claims.iter().find(|c| !ix.contains_key(**c)) {
        return Err(Fail::new("E_QUERY_INVALID", format!("{c} is not a claim"), "condorcet bundle --bundle <file>"));
    }
    let c = registry::composer("wmc-exact@1").ok_or_else(|| Fail::new("E_COMPOSER_UNKNOWN", "wmc-exact", "condorcet status"))?;
    let out = c
        .compose(&b, &json!({}), &q, &c.spec().default_budget().started())
        .map_err(|e| Fail::new(&e.code, e.message, "condorcet core --bundle <file>"))?;
    let o = &out.outputs;
    Ok(json!({
        "claims": claims,
        "joint": o["joint"]["query"],
        "conditional": o.get("conditional").map(|c| c["query"].clone()),
        "expected_false": o["expected_false"]["query"],
        "marginal": claims.iter().map(|&c| (c.to_owned(), o["marginal"][c].clone())).collect::<Map<_, _>>(),
        "partition": o["partition"],
    }))
}

/// The bundle claim a gate's `--staged` names: equal, or ending in
/// `/<staged>` or `:<staged>`.
fn staged_claim(b: &Bundle, staged: &str) -> Option<String> {
    b.claims
        .iter()
        .find(|c| c.id == staged || c.id.ends_with(&format!("/{staged}")) || c.id.ends_with(&format!(":{staged}")))
        .map(|c| c.id.clone())
}

fn gate(a: &Args) -> R {
    let next = "condorcet gate --adapter <id> --profile <name> --staged <id>";
    let staged = a.need("staged", next)?;
    let profile_name = a.need("profile", next)?;
    let (b, adapter_id, scope) = if let Some(f) = a.get("bundle") {
        (source(a)?.0, None, json!({"path": f}))
    } else {
        let id = a.need("adapter", next)?;
        let mut scope = json!({"staged": staged});
        if let Some(p) = a.get("proposed") {
            scope["proposed_path"] = json!(p);
        }
        (adapter(id)?.load(&scope)?, Some(id), scope)
    };
    let profile = resolve_profile(profile_name, adapter_id)?;
    let claim = staged_claim(&b, staged).ok_or_else(|| {
        Fail::new("E_STAGED_NOT_IN_BUNDLE", format!("the adapter's bundle has no claim for {staged}"), "condorcet bundle --adapter <id> --scope '{\"staged\":\"<id>\"}'")
    })?;
    let q = query(a, &b, Some(claim.clone()))?;
    let (r, memoised, warn) = run(&b, &profile, &q);
    let (verdict, reasons) = gate_verdict(&profile, &r, &claim);
    let pick = |composer: &str, output: &str, key: &str| -> Value {
        r.steps
            .iter()
            .find(|s| s.composer.starts_with(&format!("{composer}@")))
            .and_then(|s| s.outputs.as_ref())
            .and_then(|o| o.get(output))
            .and_then(|v| v.get(key))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let mut data = json!({
        "verdict": verdict,
        "reasons": reasons,
        "run": r.id,
        "staged": claim,
        "bundle": r.bundle,
        "profile": format!("{}@{}", r.profile.name, r.profile.version),
        "run_verdict": r.verdict,
        "gates": r.gates,
        "steps": r.steps.iter().map(|s| json!({"composer": s.composer, "status": s.status, "role": s.role})).collect::<Vec<_>>(),
        "staged_scores": {
            "marginal": pick("wmc-exact", "marginal", &claim),
            "conditional": pick("wmc-exact", "conditional", "load_bearing"),
            "joint_load_bearing": pick("wmc-exact", "joint", "load_bearing"),
            "expected_false_load_bearing": pick("wmc-exact", "expected_false", "load_bearing"),
            "label": pick("dung", "label", &claim),
            "jury_majority_correct": pick("condorcet-jury", "majority_correct", &claim),
        },
        "claims": b.claims.len(),
        "memoised": memoised,
        "scope": scope,
    });
    if let Some(w) = warn {
        data["warning"] = json!(w);
    }
    Ok(data)
}

/// pass | hold | refuse, and why. A failed gate maps by its `on_fail`
/// (default hold); an inconclusive run holds.
fn gate_verdict(profile: &Profile, r: &RunRecord, claim: &str) -> (&'static str, Vec<String>) {
    let mut refuse = false;
    let mut reasons = Vec::new();
    for (spec, rec) in profile.gates.iter().zip(&r.gates) {
        let detail = rec.detail.clone().unwrap_or_default();
        if detail.starts_with("not evaluated") {
            reasons.push(format!("{}: {detail}", rec.id));
        } else if !rec.passed {
            refuse |= spec.on_fail.as_deref() == Some("refuse");
            reasons.push(format!("{} failed: {detail}", rec.id));
        }
    }
    for s in &r.steps {
        let Some(o) = &s.outputs else {
            if let Some(e) = &s.error {
                reasons.push(format!("{} {}: {} {}", s.composer, s.status, e.code, e.message));
            }
            continue;
        };
        if let Some(core) = o.get("core").filter(|_| o.get("satisfiable") == Some(&json!(false))) {
            reasons.push(format!("unsat core: {core}"));
        }
        if let Some(chain) = o.get("attackers").and_then(|a| a.get(claim)) {
            reasons.push(format!("defeated: {chain}"));
        }
    }
    let failed = r.gates.iter().any(|g| !g.passed && !g.detail.as_deref().unwrap_or("").starts_with("not evaluated"));
    let verdict = if refuse {
        "refuse"
    } else if failed || r.verdict != "pass" {
        "hold"
    } else {
        "pass"
    };
    (verdict, reasons)
}

fn explain(id: &str) -> R {
    let store = Store::open(default_root());
    let r = store.get_run(id)?;
    let claims: Map<String, Value> = store
        .get_bundle(&r.bundle)
        .map(|b| b.claims.into_iter().map(|c| (c.id, json!(c.text))).collect())
        .unwrap_or_default();
    Ok(json!({
        "run": r.id,
        "verdict": r.verdict,
        "profile": r.profile,
        "bundle": r.bundle,
        "adapter": r.adapter,
        "created_at": r.created_at,
        "gates": r.gates,
        "steps": r.steps.iter().map(|s| json!({
            "composer": s.composer, "role": s.role, "status": s.status, "error": s.error,
            "outputs": s.outputs, "why": s.evidence,
            "assumptions": registry::spec(&s.composer).map(|v| v["assumptions"].clone()),
        })).collect::<Vec<_>>(),
        "claims": claims,
    }))
}

fn resolve(id: &str) -> R {
    let store = Store::open(default_root());
    match id.split(':').next() {
        Some("run") => Ok(json!({"kind": "run", "record": store.get_run(id)?})),
        Some("bundle") => Ok(json!({"kind": "bundle", "record": store.get_bundle(id)?})),
        Some("composer") => {
            let name = id.trim_start_matches("composer:");
            registry::spec(name).map(|v| json!({"kind": "composer", "record": v}))
                .ok_or_else(|| Fail::new("E_NOT_FOUND", format!("no {id}"), "condorcet composer list"))
        }
        Some("profile") => Ok(json!({"kind": "profile", "record": resolve_profile(id.trim_start_matches("profile:"), None)?.to_value()})),
        _ => Err(Fail::new("E_ID_INVALID", format!("{id} names no kind (run:, bundle:, composer:, profile:)"), "condorcet help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_shape() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 20);
        assert!(t.starts_with("20") && t.ends_with('Z'));
    }

    #[test]
    fn staged_claim_matching() {
        let v: Value = serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap();
        let b = Bundle::from_json(v).unwrap();
        assert_eq!(staged_claim(&b, "fct-a1").as_deref(), Some("org:clx/fct-a1"));
        assert_eq!(staged_claim(&b, "org:clx/fct-b2").as_deref(), Some("org:clx/fct-b2"));
        assert!(staged_claim(&b, "nope").is_none());
    }

    #[test]
    fn engine_runs_validate_against_the_run_schema() {
        for f in [
            include_str!("../../../schemas/examples/bundle.video-clock.json"),
            include_str!("../../../fixtures/contradictory-pair.json"),
        ] {
            let b = Bundle::from_json(serde_json::from_str(f).unwrap()).unwrap();
            for (name, _) in registry::PROFILES {
                let p = registry::builtin_profile(name).unwrap();
                let q = Query::default_for(&b, Some(b.claims[0].id.clone()));
                let mut r = run_profile(&b, &p, &q, &registry::composer);
                r.created_at = Some(now_rfc3339());
                let v = serde_json::to_value(&r).unwrap();
                assert!(schema_errors("run", &v).is_empty(), "{name}: {:?}", schema_errors("run", &v));
                assert!(schema_errors("bundle", &serde_json::to_value(&b).unwrap()).is_empty());
            }
        }
    }

    #[test]
    fn builtin_specs_and_profiles_load() {
        for id in registry::composer_ids() {
            assert!(registry::composer(&id).is_some(), "{id}");
            assert!(schema_errors("composer", &registry::spec(&id).unwrap()).is_empty(), "{id}");
        }
        for (name, text) in registry::PROFILES {
            assert!(registry::builtin_profile(name).is_some(), "{name}");
            assert!(schema_errors("profile", &serde_json::from_str(text).unwrap()).is_empty(), "{name}");
        }
    }
}
