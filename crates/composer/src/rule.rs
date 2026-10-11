//! Profile gate rules: `<composer>.<output>[(<arg>)] <op> <literal>`.
//!
//! `op` is one of `== != >= <= > <`; the literal is a number, `true`,
//! `false` or a single-quoted string. `arg` selects one entry of a keyed
//! output: a set name (`load_bearing`, `all`), a claim id, or `staged`
//! (the query's focus claim).

use condorcet_core::StepRecord;
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub composer: String,
    pub output: String,
    pub arg: Option<String>,
    pub op: Op,
    pub literal: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

const OPS: [(&str, Op); 6] = [("==", Op::Eq), ("!=", Op::Ne), (">=", Op::Ge), ("<=", Op::Le), (">", Op::Gt), ("<", Op::Lt)];

/// # Errors
/// A message naming what is malformed.
pub fn parse(rule: &str) -> Result<Rule, String> {
    let (pos, sym, op) = OPS
        .iter()
        .filter_map(|(s, o)| rule.find(s).map(|p| (p, *s, *o)))
        .min_by_key(|(p, s, _)| (*p, usize::MAX - s.len()))
        .ok_or_else(|| format!("rule {rule:?} has no comparison operator"))?;
    let lhs = rule[..pos].trim();
    let rhs = rule[pos + sym.len()..].trim();
    let (path, arg) = match lhs.split_once('(') {
        Some((p, rest)) => {
            let a = rest.strip_suffix(')').ok_or_else(|| format!("rule {rule:?}: unclosed ("))?;
            (p.trim(), Some(a.trim().trim_matches('\'').to_owned()))
        }
        None => (lhs, None),
    };
    let (composer, output) = path.rsplit_once('.').ok_or_else(|| format!("rule {rule:?}: expected <composer>.<output>"))?;
    let composer = composer.split('@').next().unwrap_or(composer);
    let literal = if let Some(s) = rhs.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Value::String(s.to_owned())
    } else {
        serde_json::from_str::<Value>(rhs).map_err(|_| format!("rule {rule:?}: bad literal {rhs}"))?
    };
    if composer.is_empty() || output.is_empty() {
        return Err(format!("rule {rule:?}: empty composer or output"));
    }
    Ok(Rule { composer: composer.to_owned(), output: output.to_owned(), arg, op, literal })
}

fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// Look the rule's value up in the run's steps. `Err` says why the rule
/// could not be evaluated (the step did not run, the output is absent).
///
/// # Errors
/// The value the rule names is not available.
pub fn lookup(rule: &Rule, steps: &[StepRecord], focus: Option<&str>) -> Result<Value, String> {
    let step = steps
        .iter()
        .find(|s| s.composer.split('@').next() == Some(rule.composer.as_str()))
        .ok_or_else(|| format!("{} is not in the profile", rule.composer))?;
    if step.status != "ok" {
        return Err(format!("{} {}", rule.composer, step.status));
    }
    let v = step
        .outputs
        .as_ref()
        .and_then(|o| o.get(&rule.output))
        .ok_or_else(|| format!("{} has no output {}", rule.composer, rule.output))?;
    let Some(arg) = &rule.arg else { return Ok(v.clone()) };
    let resolved = if arg == "staged" { focus } else { None };
    v.get(arg.as_str())
        .or_else(|| resolved.and_then(|f| v.get(f)))
        .cloned()
        .ok_or_else(|| format!("{}.{} has no entry {arg}", rule.composer, rule.output))
}

/// Evaluate: `Ok(passed, detail)`, or `Err` when it cannot be evaluated.
///
/// # Errors
/// The value is unavailable or not comparable with the literal.
pub fn eval(rule: &Rule, steps: &[StepRecord], focus: Option<&str>) -> Result<(bool, String), String> {
    let v = lookup(rule, steps, focus)?;
    let ord = compare(&v, &rule.literal).ok_or_else(|| format!("cannot compare {v} with {}", rule.literal))?;
    let passed = match rule.op {
        Op::Eq => ord == Ordering::Equal,
        Op::Ne => ord != Ordering::Equal,
        Op::Ge => ord != Ordering::Less,
        Op::Le => ord != Ordering::Greater,
        Op::Gt => ord == Ordering::Greater,
        Op::Lt => ord == Ordering::Less,
    };
    let lhs = match &rule.arg {
        Some(a) => format!("{}.{}({a})", rule.composer, rule.output),
        None => format!("{}.{}", rule.composer, rule.output),
    };
    Ok((passed, format!("{lhs}={v}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(composer: &str, outputs: Value) -> StepRecord {
        StepRecord {
            composer: composer.into(),
            role: None,
            status: "ok".into(),
            params: None,
            outputs: outputs.as_object().cloned(),
            evidence: vec![],
            elapsed_ms: None,
            error: None,
        }
    }

    #[test]
    fn parses_the_design_examples() {
        let r = parse("sat-core.satisfiable == true").unwrap();
        assert_eq!((r.composer.as_str(), r.output.as_str(), r.op, r.literal.clone()), ("sat-core", "satisfiable", Op::Eq, json!(true)));
        let r = parse("dung.label(staged) != 'out'").unwrap();
        assert_eq!((r.arg.as_deref(), r.op, r.literal.clone()), (Some("staged"), Op::Ne, json!("out")));
        let r = parse("wmc-exact.joint(load_bearing) >= 0.5").unwrap();
        assert_eq!((r.op, r.literal.clone()), (Op::Ge, json!(0.5)));
        assert!(parse("no operator").is_err());
        assert!(parse("x == nope").is_err());
    }

    #[test]
    fn evaluates_against_steps_with_focus() {
        let steps = vec![
            step("sat-core@1", json!({"satisfiable": true})),
            step("dung@1", json!({"label": {"org:a": "out"}})),
            step("wmc-exact@1", json!({"joint": {"load_bearing": 0.31}})),
        ];
        let e = |r: &str| eval(&parse(r).unwrap(), &steps, Some("org:a"));
        assert!(e("sat-core.satisfiable == true").unwrap().0);
        assert!(!e("dung.label(staged) != 'out'").unwrap().0);
        let (ok, detail) = e("wmc-exact.joint(load_bearing) >= 0.5").unwrap();
        assert!(!ok);
        assert_eq!(detail, "wmc-exact.joint(load_bearing)=0.31");
        assert!(e("dung@1.label(staged) == 'out'").unwrap().0);
        assert!(e("psl.degree(staged) > 0").is_err());
    }
}
