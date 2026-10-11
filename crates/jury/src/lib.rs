//! Composer `condorcet-jury@1`: the Condorcet jury theorem (Condorcet 1785)
//! over a bundle. Each evidential reading of a claim is a juror voting for
//! it (support) or against it (contradict), correct with probability equal
//! to the link's reliability. The output is the probability that the
//! majority vote is correct (a Poisson-binomial tail by dynamic
//! programming; a tie is broken by a fair coin).
//!
//! Correlated jurors break the theorem: members of a declared cluster with
//! `rho > 0` are collapsed to one juror (mean competence, majority vote),
//! the uncollapsed figure is reported beside it, and the ceiling
//! `rho * p + (1 - rho)` that more correlated jurors cannot pass is always
//! emitted. Jurors with competence <= 0.5 carry no positive information
//! and are dropped and reported.

use condorcet_composer::{known_params, param_enum, Budget, ComposeError, Composer, ComposerSpec, Output, Query};
use condorcet_core::{Bundle, Evidence, LinkKind};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const SPEC: &str = include_str!("../../../composers/condorcet-jury/composer.json");

/// Probability the majority of independent jurors with competences `p` is
/// correct; ties (even n) count one half. Empty jury: 0.5.
#[must_use]
pub fn majority_correct(p: &[f64]) -> f64 {
    let n = p.len();
    let mut dist = vec![0.0; n + 1];
    dist[0] = 1.0;
    for (i, &pi) in p.iter().enumerate() {
        for k in (0..=i + 1).rev() {
            let stay = dist[k] * (1.0 - pi);
            let up = if k > 0 { dist[k - 1] * pi } else { 0.0 };
            dist[k] = stay + up;
        }
    }
    dist.iter()
        .enumerate()
        .map(|(k, &d)| match (2 * k).cmp(&n) {
            std::cmp::Ordering::Greater => d,
            std::cmp::Ordering::Equal => d / 2.0,
            std::cmp::Ordering::Less => 0.0,
        })
        .sum()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Juror {
    pub ids: Vec<String>,
    pub competence: f64,
    /// `true`: votes the claim true.
    pub vote: bool,
}

struct ClaimJury {
    raw: Vec<Juror>,
    jurors: Vec<Juror>,
    dropped: Vec<String>,
    ceiling: f64,
}

fn cluster_of<'a>(b: &'a Bundle, from: &str) -> Option<(&'a str, f64)> {
    let ex = b.exhibits.iter().find(|e| e.id == from)?;
    ex.clusters.iter().find_map(|k| b.clusters.iter().find(|g| &g.key == k && g.rho > 0.0).map(|g| (g.key.as_str(), g.rho)))
}

fn jury_for(b: &Bundle, claim: &str, collapse: bool) -> ClaimJury {
    let mut raw = Vec::new();
    let mut dropped = Vec::new();
    let mut groups: BTreeMap<&str, (f64, Vec<Juror>)> = BTreeMap::new();
    let mut singles = Vec::new();
    for l in b.links.iter().filter(|l| l.to == claim && l.kind != LinkKind::Undercut) {
        let Some(p) = l.weight.and_then(condorcet_core::Weight::reliability) else {
            dropped.push(l.id.clone());
            continue;
        };
        if p <= 0.5 {
            dropped.push(l.id.clone());
            continue;
        }
        let j = Juror { ids: vec![l.id.clone()], competence: p, vote: l.kind == LinkKind::Support };
        raw.push(j.clone());
        match cluster_of(b, &l.from) {
            Some((k, rho)) if collapse => groups.entry(k).or_insert((rho, vec![])).1.push(j),
            _ => singles.push(j),
        }
    }
    let mut ceiling: f64 = 1.0;
    let mut jurors = singles;
    for (rho, members) in groups.into_values() {
        #[allow(clippy::cast_precision_loss)]
        let mean = members.iter().map(|j| j.competence).sum::<f64>() / members.len() as f64;
        if members.len() > 1 {
            ceiling = ceiling.min(rho.mul_add(mean, 1.0 - rho));
        }
        let yes = members.iter().filter(|j| j.vote).count();
        let ids: Vec<String> = members.iter().flat_map(|j| j.ids.clone()).collect();
        match (2 * yes).cmp(&members.len()) {
            std::cmp::Ordering::Equal => dropped.extend(ids),
            o => jurors.push(Juror { ids, competence: mean, vote: o == std::cmp::Ordering::Greater }),
        }
    }
    ClaimJury { raw, jurors, dropped, ceiling }
}

pub struct Jury {
    spec: ComposerSpec,
}

impl Default for Jury {
    fn default() -> Self {
        Self { spec: ComposerSpec::builtin(SPEC) }
    }
}

fn competences(js: &[Juror]) -> Vec<f64> {
    js.iter().map(|j| j.competence).collect()
}

impl Composer for Jury {
    fn spec(&self) -> &ComposerSpec {
        &self.spec
    }

    fn compose(&self, b: &Bundle, params: &Value, _q: &Query, budget: &Budget) -> Result<Output, ComposeError> {
        known_params(params, &self.spec)?;
        let collapse = param_enum(params, "clusters", &["collapse", "ignore"], "collapse")? == "collapse";
        budget.check_vars(b.links.len())?;
        let mut out = Output::default();
        let (mut mc, mut raw, mut ceil, mut maj, mut n) = (Map::new(), Map::new(), Map::new(), Map::new(), Map::new());
        for c in &b.claims {
            let j = jury_for(b, &c.id, collapse);
            if j.raw.is_empty() && j.dropped.is_empty() {
                continue;
            }
            let yes = j.jurors.iter().filter(|x| x.vote).count();
            let majority = match (2 * yes).cmp(&j.jurors.len()) {
                std::cmp::Ordering::Greater => "true",
                std::cmp::Ordering::Less => "false",
                std::cmp::Ordering::Equal => "tie",
            };
            mc.insert(c.id.clone(), json!(majority_correct(&competences(&j.jurors))));
            raw.insert(c.id.clone(), json!(majority_correct(&competences(&j.raw))));
            ceil.insert(c.id.clone(), json!(j.ceiling));
            maj.insert(c.id.clone(), json!(majority));
            n.insert(c.id.clone(), json!(j.jurors.len()));
            if !j.dropped.is_empty() {
                out.evidence.push(Evidence::new("note", j.dropped, Some(json!({"claim": c.id, "dropped": "competence <= 0.5, unweighted, or a tied cluster"}))));
            }
        }
        out.outputs.insert("majority_correct".into(), Value::Object(mc));
        out.outputs.insert("raw_majority_correct".into(), Value::Object(raw));
        out.outputs.insert("ceiling".into(), Value::Object(ceil));
        out.outputs.insert("majority".into(), Value::Object(maj));
        out.outputs.insert("jurors".into(), Value::Object(n));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_values_from_the_design_note() {
        assert!((majority_correct(&[0.6; 3]) - 0.648).abs() < 1e-12);
        assert!((majority_correct(&[0.6; 5]) - 0.682_56).abs() < 1e-12);
        assert!((majority_correct(&[0.6]) - 0.6).abs() < 1e-12);
        assert!((majority_correct(&[]) - 0.5).abs() < 1e-12);
    }
}
