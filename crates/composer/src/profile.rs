//! Profiles (schema `condorcet/profile/v1`) and the cascade runner.
//!
//! Steps run in order. A step with `on_fail: stop` that fails, or reaches a
//! negative decision (UNSAT), ends the gate cascade: later gate steps are
//! `skipped`. `role: baseline` steps always run and never gate.

use crate::{rule, Budget, Composer, Query};
use condorcet_core::{content_id, Bundle, GateRecord, IdKind, NameVersion, RunRecord, StepError, StepRecord, RUN_SCHEMA};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub composer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl Step {
    fn stops(&self) -> bool {
        self.on_fail.as_deref() == Some("stop")
    }

    fn is_baseline(&self) -> bool {
        self.role.as_deref() == Some("baseline")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateSpec {
    pub id: String,
    pub rule: String,
    /// `refuse` or `hold` (default): how a failure maps onto a gate verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub version: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub gates: Vec<GateSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Profile {
    /// Parse and check a profile: at least one step, composer ids of the
    /// form `name@N`, enum fields in range, every gate rule parses.
    ///
    /// # Errors
    /// `(code, message)`: `E_PROFILE_INVALID`.
    pub fn from_json(v: Value) -> Result<Self, (String, String)> {
        let bad = |m: String| ("E_PROFILE_INVALID".to_owned(), m);
        let p: Self = serde_json::from_value(v).map_err(|e| bad(e.to_string()))?;
        if p.steps.is_empty() {
            return Err(bad("a profile needs at least one step".into()));
        }
        for s in &p.steps {
            let ok_id = s.composer.split_once('@').is_some_and(|(n, v)| {
                !n.is_empty() && !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())
            });
            if !ok_id {
                return Err(bad(format!("step composer {:?} must be name@N", s.composer)));
            }
            if !matches!(s.on_fail.as_deref(), None | Some("stop" | "continue")) {
                return Err(bad(format!("step {}: on_fail must be stop|continue", s.composer)));
            }
            if !matches!(s.role.as_deref(), None | Some("gate" | "baseline")) {
                return Err(bad(format!("step {}: role must be gate|baseline", s.composer)));
            }
        }
        for g in &p.gates {
            rule::parse(&g.rule).map_err(bad)?;
            if !matches!(g.on_fail.as_deref(), None | Some("hold" | "refuse")) {
                return Err(bad(format!("gate {}: on_fail must be hold|refuse", g.id)));
            }
        }
        Ok(p)
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Looks up a composer by `name@version`.
pub type Registry<'a> = dyn Fn(&str) -> Option<Box<dyn Composer>> + 'a;

fn step_record(step: &Step, status: &str) -> StepRecord {
    StepRecord {
        composer: step.composer.clone(),
        role: Some(step.role.clone().unwrap_or_else(|| "gate".into())),
        status: status.into(),
        params: step.params.clone(),
        outputs: None,
        evidence: vec![],
        elapsed_ms: None,
        error: None,
    }
}

fn run_step(step: &Step, b: &Bundle, q: &Query, registry: &Registry<'_>) -> (StepRecord, bool) {
    let mut rec = step_record(step, "ok");
    let Some(composer) = registry(&step.composer) else {
        rec.status = "failed".into();
        rec.error = Some(StepError {
            code: "E_COMPOSER_UNKNOWN".into(),
            message: format!("no composer {} in this build", step.composer),
            next: Some("condorcet composer list".into()),
        });
        return (rec, true);
    };
    let budget = composer
        .spec()
        .default_budget()
        .overlay(&step.budget.as_ref().map(Budget::from_json).unwrap_or_default())
        .started();
    let params = step.params.clone().unwrap_or(Value::Object(serde_json::Map::new()));
    let t0 = Instant::now();
    let result = composer.compose(b, &params, q, &budget);
    rec.elapsed_ms = Some(u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX));
    match result {
        Ok(out) => {
            rec.outputs = Some(out.outputs);
            rec.evidence = out.evidence;
            (rec, out.failed_decision)
        }
        Err(e) => {
            rec.status = if e.budget_exceeded { "budget_exceeded" } else { "failed" }.into();
            rec.error = Some(StepError { code: e.code, message: e.message, next: Some(format!("condorcet composer show {}", composer.spec().name)) });
            (rec, true)
        }
    }
}

/// Run `profile` over `b`. The record is sealed (content id set);
/// `created_at` is left for the caller.
#[must_use]
pub fn run_profile(b: &Bundle, profile: &Profile, q: &Query, registry: &Registry<'_>) -> RunRecord {
    let mut steps = Vec::new();
    let mut stopped = false;
    for step in &profile.steps {
        if stopped && !step.is_baseline() {
            steps.push(step_record(step, "skipped"));
            continue;
        }
        let (rec, failed) = run_step(step, b, q, registry);
        stopped |= failed && step.stops() && !step.is_baseline();
        steps.push(rec);
    }
    let gate_steps: Vec<StepRecord> = steps.iter().filter(|s| s.role.as_deref() != Some("baseline")).cloned().collect();
    let gates: Vec<(GateRecord, bool)> = profile
        .gates
        .iter()
        .map(|g| match rule::parse(&g.rule).and_then(|r| rule::eval(&r, &gate_steps, q.focus.as_deref())) {
            Ok((passed, detail)) => (GateRecord { id: g.id.clone(), passed, detail: Some(detail) }, true),
            Err(why) => (GateRecord { id: g.id.clone(), passed: false, detail: Some(format!("not evaluated: {why}")) }, false),
        })
        .collect();
    let any_failed = gates.iter().any(|(g, evaluated)| *evaluated && !g.passed);
    let all_evaluated = gates.iter().all(|(_, e)| *e);
    let verdict = if any_failed {
        "fail"
    } else if all_evaluated {
        "pass"
    } else {
        "inconclusive"
    };
    let bundle_id = content_id(IdKind::Bundle, &serde_json::to_value(b).unwrap_or(Value::Null));
    let composers: Vec<String> = steps.iter().filter(|s| s.status != "skipped").map(|s| s.composer.clone()).collect();
    let mut rec = RunRecord {
        schema: RUN_SCHEMA.into(),
        id: String::new(),
        memo_key: condorcet_core::run::memo_key(&bundle_id, &profile.to_value(), &composers, profile.seed),
        bundle: bundle_id,
        adapter: Some(b.adapter.clone()),
        profile: NameVersion { name: profile.name.clone(), version: profile.version.clone() },
        composers,
        seed: profile.seed,
        created_at: None,
        steps,
        gates: gates.into_iter().map(|(g, _)| g).collect(),
        verdict: verdict.into(),
    };
    rec.seal();
    rec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComposeError, ComposerSpec, Output};
    use serde_json::json;

    struct Fixed {
        spec: ComposerSpec,
        out: Result<Value, ()>,
        fail: bool,
    }

    impl Composer for Fixed {
        fn spec(&self) -> &ComposerSpec {
            &self.spec
        }
        fn compose(&self, _: &Bundle, _: &Value, _: &Query, _: &Budget) -> Result<Output, ComposeError> {
            match &self.out {
                Ok(v) => Ok(Output { outputs: v.as_object().cloned().unwrap_or_default(), evidence: vec![], failed_decision: self.fail }),
                Err(()) => Err(ComposeError::new("E_X", "boom")),
            }
        }
    }

    fn fixed(name: &str, out: Result<Value, ()>, fail: bool) -> Box<dyn Composer> {
        let spec = ComposerSpec { name: name.into(), version: "1".into(), declared: json!({"name": name, "version": "1"}) };
        Box::new(Fixed { spec, out, fail })
    }

    fn bundle() -> Bundle {
        Bundle::from_json(serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap()).unwrap()
    }

    fn registry(sat: bool) -> impl Fn(&str) -> Option<Box<dyn Composer>> {
        move |id: &str| match id {
            "sat-core@1" => Some(fixed("sat-core", Ok(json!({"satisfiable": sat})), !sat)),
            "wmc-exact@1" => Some(fixed("wmc-exact", Ok(json!({"joint": {"load_bearing": 0.7}})), false)),
            "base@1" => Some(fixed("base", Ok(json!({"x": 1})), false)),
            "broken@1" => Some(fixed("broken", Err(()), false)),
            _ => None,
        }
    }

    fn profile() -> Profile {
        Profile::from_json(json!({
            "name": "t", "version": "1",
            "steps": [{"composer": "sat-core@1", "on_fail": "stop"}, {"composer": "wmc-exact@1"}, {"composer": "base@1", "role": "baseline"}],
            "gates": [{"id": "c", "rule": "sat-core.satisfiable == true", "on_fail": "refuse"}, {"id": "j", "rule": "wmc-exact.joint(load_bearing) >= 0.5"}]
        }))
        .unwrap()
    }

    #[test]
    fn passing_cascade_runs_every_step_and_seals() {
        let b = bundle();
        let r = run_profile(&b, &profile(), &Query::default_for(&b, None), &registry(true));
        assert_eq!(r.verdict, "pass");
        assert_eq!(r.composers, vec!["sat-core@1", "wmc-exact@1", "base@1"]);
        assert!(r.id.starts_with("run:s256:"));
        let again = run_profile(&b, &profile(), &Query::default_for(&b, None), &registry(true));
        assert_eq!((again.id, again.memo_key), (r.id, r.memo_key));
    }

    #[test]
    fn unsat_stops_gate_steps_but_not_baselines() {
        let b = bundle();
        let r = run_profile(&b, &profile(), &Query::default_for(&b, None), &registry(false));
        let st: Vec<&str> = r.steps.iter().map(|s| s.status.as_str()).collect();
        assert_eq!(st, vec!["ok", "skipped", "ok"]);
        assert_eq!(r.verdict, "fail");
        assert!(!r.gates[0].passed);
        assert!(r.gates[1].detail.as_deref().unwrap().starts_with("not evaluated"));
    }

    #[test]
    fn unknown_and_failing_composers_are_recorded_not_fatal() {
        let b = bundle();
        let p = Profile::from_json(json!({"name": "t", "version": "1", "steps": [{"composer": "nope@1"}, {"composer": "broken@1"}],
            "gates": [{"id": "g", "rule": "broken.x == 1"}]})).unwrap();
        let r = run_profile(&b, &p, &Query::default_for(&b, None), &registry(true));
        assert_eq!(r.steps[0].error.as_ref().unwrap().code, "E_COMPOSER_UNKNOWN");
        assert_eq!(r.steps[1].status, "failed");
        assert_eq!(r.verdict, "inconclusive");
    }

    #[test]
    fn profile_validation() {
        assert!(Profile::from_json(json!({"name": "t", "version": "1", "steps": []})).is_err());
        assert!(Profile::from_json(json!({"name": "t", "version": "1", "steps": [{"composer": "x"}]})).is_err());
        assert!(Profile::from_json(json!({"name": "t", "version": "1", "steps": [{"composer": "x@1"}], "gates": [{"id": "g", "rule": "bad"}]})).is_err());
        let ex: Value = serde_json::from_str(include_str!("../../../schemas/examples/profile.gate-strict.json")).unwrap();
        assert!(Profile::from_json(ex).is_ok());
    }
}
