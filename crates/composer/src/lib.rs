//! Composer trait, the declared composer spec, profiles, the cascade runner
//! and the gate-rule evaluator. Composers are pure functions of
//! `(Bundle, Params, Query, Budget)`.

pub mod profile;
pub mod rule;

use condorcet_core::{Bundle, Evidence};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::Instant;

pub use profile::{memo_key, run_profile, GateSpec, Profile, Registry, Step};

/// A composer's declaration, `composers/<name>/composer.json`.
#[derive(Debug, Clone)]
pub struct ComposerSpec {
    pub name: String,
    pub version: String,
    pub declared: Value,
}

impl ComposerSpec {
    /// Load a declaration; it must name itself and its version.
    ///
    /// # Panics
    /// When a built-in declaration is malformed (a build defect, caught by tests).
    #[must_use]
    pub fn builtin(json: &str) -> Self {
        let declared: Value = serde_json::from_str(json).expect("built-in composer.json parses");
        let field = |k: &str| declared[k].as_str().expect("composer.json names itself").to_owned();
        Self { name: field("name"), version: field("version"), declared }
    }

    /// `name@version`.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }

    /// The declared default budget.
    #[must_use]
    pub fn default_budget(&self) -> Budget {
        Budget::from_json(&self.declared["cost"]["budget"])
    }
}

/// What the caller asks about: named claim sets and an optional focus claim
/// (the staged fact in a gate).
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub sets: BTreeMap<String, Vec<String>>,
    pub focus: Option<String>,
}

impl Query {
    /// The default query: `load_bearing` = every claim that is not an
    /// undercutting defeater; `all` = every claim.
    #[must_use]
    pub fn default_for(b: &Bundle, focus: Option<String>) -> Self {
        let defeaters = b.defeaters();
        let lb = b.claims.iter().filter(|c| !defeaters.contains(c.id.as_str())).map(|c| c.id.clone()).collect();
        let mut sets = BTreeMap::new();
        sets.insert("load_bearing".to_owned(), lb);
        sets.insert("all".to_owned(), b.claims.iter().map(|c| c.id.clone()).collect());
        Self { sets, focus }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Budget {
    pub max_vars: Option<usize>,
    pub timeout_ms: Option<u64>,
    pub max_cores: Option<usize>,
    pub deadline: Option<Instant>,
}

impl Budget {
    #[must_use]
    pub fn from_json(v: &Value) -> Self {
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        Self {
            max_vars: n("max_vars").and_then(|x| usize::try_from(x).ok()),
            timeout_ms: n("timeout_ms"),
            max_cores: n("max_cores").and_then(|x| usize::try_from(x).ok()),
            deadline: None,
        }
    }

    /// `over` wins field by field.
    #[must_use]
    pub fn overlay(&self, over: &Self) -> Self {
        Self {
            max_vars: over.max_vars.or(self.max_vars),
            timeout_ms: over.timeout_ms.or(self.timeout_ms),
            max_cores: over.max_cores.or(self.max_cores),
            deadline: over.deadline.or(self.deadline),
        }
    }

    /// Start the clock.
    #[must_use]
    pub fn started(mut self) -> Self {
        self.deadline = self.timeout_ms.map(|ms| Instant::now() + std::time::Duration::from_millis(ms));
        self
    }

    /// # Errors
    /// `E_BUDGET_EXCEEDED` past the deadline.
    pub fn check_time(&self) -> Result<(), ComposeError> {
        match self.deadline {
            Some(d) if Instant::now() > d => Err(ComposeError::budget("timeout exceeded")),
            _ => Ok(()),
        }
    }

    /// # Errors
    /// `E_BUDGET_EXCEEDED` when `n` variables exceed `max_vars`.
    pub fn check_vars(&self, n: usize) -> Result<(), ComposeError> {
        match self.max_vars {
            Some(m) if n > m => Err(ComposeError::budget(&format!("{n} variables > max_vars {m}"))),
            _ => Ok(()),
        }
    }
}

/// A composer's result: outputs keyed by declared output name, and why.
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub outputs: Map<String, Value>,
    pub evidence: Vec<Evidence>,
    /// The step reached a negative decision (e.g. UNSAT); `on_fail: stop`
    /// ends the cascade.
    pub failed_decision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeError {
    pub code: String,
    pub message: String,
    pub budget_exceeded: bool,
}

impl ComposeError {
    #[must_use]
    pub fn new(code: &str, message: &str) -> Self {
        Self { code: code.into(), message: message.into(), budget_exceeded: false }
    }

    #[must_use]
    pub fn budget(message: &str) -> Self {
        Self { code: "E_BUDGET_EXCEEDED".into(), message: message.into(), budget_exceeded: true }
    }
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ComposeError {}

pub trait Composer {
    fn spec(&self) -> &ComposerSpec;
    /// # Errors
    /// Bad params, unsupported input, or the budget ran out.
    fn compose(&self, b: &Bundle, params: &Value, q: &Query, budget: &Budget) -> Result<Output, ComposeError>;
}

/// Read an optional string param with a default, checked against `allowed`.
///
/// # Errors
/// `E_PARAMS_INVALID`.
pub fn param_enum<'a>(params: &'a Value, key: &str, allowed: &[&'a str], default: &'a str) -> Result<&'a str, ComposeError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(s),
        Some(v) => Err(ComposeError::new("E_PARAMS_INVALID", &format!("{key} must be one of {allowed:?}, got {v}"))),
    }
}

/// Read an optional number param in `[lo, hi]`.
///
/// # Errors
/// `E_PARAMS_INVALID`.
pub fn param_f64(params: &Value, key: &str, lo: f64, hi: f64, default: f64) -> Result<f64, ComposeError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|x| (lo..=hi).contains(x))
            .ok_or_else(|| ComposeError::new("E_PARAMS_INVALID", &format!("{key} must be a number in [{lo}, {hi}]"))),
    }
}

/// Reject param keys a composer does not declare.
///
/// # Errors
/// `E_PARAMS_INVALID`.
pub fn known_params(params: &Value, spec: &ComposerSpec) -> Result<(), ComposeError> {
    let Some(obj) = params.as_object() else {
        return if params.is_null() { Ok(()) } else { Err(ComposeError::new("E_PARAMS_INVALID", "params must be an object")) };
    };
    let declared = &spec.declared["params"]["properties"];
    match obj.keys().find(|k| declared.get(k.as_str()).is_none()) {
        Some(k) => Err(ComposeError::new("E_PARAMS_INVALID", &format!("{} has no param {k}", spec.id()))),
        None => Ok(()),
    }
}
