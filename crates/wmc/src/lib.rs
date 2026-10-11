//! Composer `wmc-exact@1`: exact marginal, joint and conditional
//! probabilities by weighted model counting over encoding S:
//! `P(Q) = WMC(S and Q) / WMC(S)`.

pub mod counter;
pub mod encode;

use condorcet_composer::{known_params, param_enum, param_f64, Budget, ComposeError, Composer, ComposerSpec, Output, Query};
use condorcet_core::{Bundle, Evidence};
use counter::{Clause, Counter};
use encode::Encoding;
use serde_json::{json, Map, Value};

pub const SPEC: &str = include_str!("../../../composers/wmc-exact/composer.json");

/// Exact probabilities over one encoding, sharing the counter's cache.
pub struct Model<'a> {
    pub enc: &'a Encoding,
    counter: Counter<'a>,
    pub partition: f64,
}

impl<'a> Model<'a> {
    /// # Errors
    /// Budget exhausted.
    pub fn new(enc: &'a Encoding, budget: &'a Budget) -> Result<Self, ComposeError> {
        let mut counter = Counter::new(&enc.pos, &enc.neg, budget);
        let partition = counter.count_all(&enc.clauses, enc.num_vars())?;
        Ok(Self { enc, counter, partition })
    }

    /// `WMC(S and every claim in `claims` true) / Z`.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn joint(&mut self, claims: &[usize]) -> Result<f64, ComposeError> {
        let mut cs: Vec<Clause> = self.enc.clauses.clone();
        cs.extend(claims.iter().map(|&c| vec![Encoding::lit(self.enc.claim_var[c], true)]));
        Ok(self.counter.count_all(&cs, self.enc.num_vars())? / self.partition)
    }

    #[must_use]
    pub fn stats(&self) -> &counter::Stats {
        &self.counter.stats
    }
}

pub struct WmcExact {
    spec: ComposerSpec,
}

impl Default for WmcExact {
    fn default() -> Self {
        Self { spec: ComposerSpec::builtin(SPEC) }
    }
}

fn indices(b: &Bundle, ids: &[String]) -> Result<Vec<usize>, ComposeError> {
    let ix = b.claim_index();
    ids.iter()
        .map(|id| ix.get(id.as_str()).copied().ok_or_else(|| ComposeError::new("E_QUERY_INVALID", &format!("{id} is not a claim"))))
        .collect()
}

impl Composer for WmcExact {
    fn spec(&self) -> &ComposerSpec {
        &self.spec
    }

    fn compose(&self, b: &Bundle, params: &Value, q: &Query, budget: &Budget) -> Result<Output, ComposeError> {
        known_params(params, &self.spec)?;
        param_enum(params, "encoding", &["source-reliability"], "source-reliability")?;
        let base_rate = param_f64(params, "base_rate", 0.0, 1.0, 0.5)?;
        let ignore = param_enum(params, "unweighted", &["error", "ignore"], "error")? == "ignore";
        let enc = encode::encode(b, base_rate, ignore)?;
        budget.check_vars(enc.num_vars())?;
        let mut m = Model::new(&enc, budget)?;
        if m.partition <= 0.0 {
            return Err(ComposeError::new(
                "E_ZERO_PARTITION",
                "no world satisfies the hard constraints; run sat-core for the core",
            ));
        }
        let mut marginal = Map::new();
        let mut marg = Vec::with_capacity(b.claims.len());
        for (i, c) in b.claims.iter().enumerate() {
            let p = m.joint(&[i])?;
            marg.push(p);
            marginal.insert(c.id.clone(), json!(p));
        }
        let default_q;
        let q = if q.sets.is_empty() {
            default_q = Query::default_for(b, q.focus.clone());
            &default_q
        } else {
            q
        };
        let focus = q.focus.as_ref().map(|f| indices(b, std::slice::from_ref(f))).transpose()?.map(|v| v[0]);
        let (mut joint, mut conditional, mut expected_false) = (Map::new(), Map::new(), Map::new());
        for (name, ids) in &q.sets {
            let set = indices(b, ids)?;
            let pj = m.joint(&set)?;
            joint.insert(name.clone(), json!(pj));
            expected_false.insert(name.clone(), json!(set.iter().map(|&i| 1.0 - marg[i]).sum::<f64>()));
            if let Some(f) = focus {
                let rest: Vec<usize> = set.iter().copied().filter(|&i| i != f).collect();
                let with: Vec<usize> = rest.iter().copied().chain(std::iter::once(f)).collect();
                let den = m.joint(&rest)?;
                let num = if set.contains(&f) { pj } else { m.joint(&with)? };
                conditional.insert(name.clone(), if den > 0.0 { json!(num / den) } else { Value::Null });
            }
        }
        let tw = counter::treewidth_upper_bound(&enc.clauses, enc.num_vars());
        let mut out = Output::default();
        out.outputs.insert("marginal".into(), Value::Object(marginal));
        out.outputs.insert("joint".into(), Value::Object(joint));
        if focus.is_some() {
            out.outputs.insert("conditional".into(), Value::Object(conditional));
        }
        out.outputs.insert("expected_false".into(), Value::Object(expected_false));
        out.outputs.insert("partition".into(), json!(m.partition));
        let st = m.stats();
        out.evidence.push(Evidence::new(
            "circuit",
            vec![],
            Some(json!({"vars": enc.num_vars(), "clauses": enc.clauses.len(), "treewidth_upper_bound": tw,
                        "decisions": st.decisions, "cache_entries": st.cache_entries, "cache_hits": st.cache_hits})),
        ));
        if !enc.ignored.is_empty() {
            out.evidence.push(Evidence::new("note", enc.ignored.clone(), Some(json!("unweighted links ignored"))));
        }
        Ok(out)
    }
}
