//! Composer `sat-core@1`: can every asserted claim and hard relation hold
//! together, and if not, which smallest subsets cannot (MUSes)?
//!
//! Selector encoding: each element (an asserted claim's unit clause, a hard
//! constraint, a hard claim-to-claim link) gets a selector `s`; its clauses
//! are guarded `!s | clause`. Solving under all selectors and reading the
//! core gives an unsatisfiable subset, shrunk to minimal by deletion with
//! clause-set refinement. More cores come from MARCO enumeration.

pub mod solver;

use condorcet_composer::{known_params, param_enum, Budget, ComposeError, Composer, ComposerSpec, Output, Query};
use condorcet_core::{Bundle, Evidence, LinkKind, Mode, Weight};
use serde_json::{json, Value};
use solver::{Sat, L};
use std::collections::BTreeSet;

pub const SPEC: &str = include_str!("../../../composers/sat-core/composer.json");

/// A selectable group of clauses over claim variables.
#[derive(Debug, Clone)]
pub struct Element {
    pub id: String,
    pub clauses: Vec<Vec<(usize, bool)>>,
}

/// The elements of a bundle under `assume` (`all` | `noticed`).
#[must_use]
pub fn elements(b: &Bundle, assume: &str) -> Vec<Element> {
    let ix = b.claim_index();
    let at = |s: &str| ix[s];
    let defeaters = b.defeaters();
    let claims = b.claims.iter().enumerate().filter(|(_, c)| {
        c.noticed || (assume == "all" && !defeaters.contains(c.id.as_str()))
    });
    let units = claims.map(|(i, c)| Element { id: c.id.clone(), clauses: vec![vec![(i, true)]] });
    let constraints = b
        .constraints
        .iter()
        .filter(|k| k.mode == Mode::Hard)
        .map(|k| Element { id: k.id.clone(), clauses: k.clauses(&at) });
    let links = b
        .links
        .iter()
        .filter(|l| l.weight == Some(Weight::Unweighted(true)) && ix.contains_key(l.from.as_str()))
        .filter_map(|l| {
            let (f, t) = (ix[l.from.as_str()], ix[l.to.as_str()]);
            match l.kind {
                LinkKind::Support => Some(vec![(f, false), (t, true)]),
                LinkKind::Contradict => Some(vec![(f, false), (t, false)]),
                LinkKind::Undercut => None,
            }
            .map(|c| Element { id: l.id.clone(), clauses: vec![c] })
        });
    units.chain(constraints).chain(links).collect()
}

/// A SAT instance with one selector per element.
pub struct Instance {
    sat: Sat,
    n_claims: u32,
    selectors: Vec<u32>,
}

impl Instance {
    #[must_use]
    pub fn new(n_claims: usize, elements: &[Element]) -> Self {
        let mut sat = Sat::new();
        let n = u32::try_from(n_claims).unwrap_or(u32::MAX);
        for _ in 0..n {
            sat.var();
        }
        let selectors = elements
            .iter()
            .map(|e| {
                let s = sat.var();
                for c in &e.clauses {
                    let mut cl: Vec<L> = vec![(s, false)];
                    cl.extend(c.iter().map(|&(v, p)| (u32::try_from(v).unwrap_or(u32::MAX), p)));
                    sat.add(&cl);
                }
                s
            })
            .collect();
        Self { sat, n_claims: n, selectors }
    }

    /// Is the subset `on` (element indices) satisfiable?
    pub fn check(&mut self, on: &BTreeSet<usize>) -> bool {
        let a: Vec<L> = self.selectors.iter().enumerate().map(|(i, &s)| (s, on.contains(&i))).collect();
        self.sat.solve(&a)
    }

    /// The elements in the last core (must follow a failed `check`).
    fn core(&mut self) -> BTreeSet<usize> {
        let vars = self.sat.core_vars();
        self.selectors.iter().enumerate().filter(|(_, s)| vars.contains(s)).map(|(i, _)| i).collect()
    }

    /// Claim truth values of the last model.
    #[must_use]
    pub fn model(&self) -> Vec<bool> {
        (0..self.n_claims).map(|v| self.sat.value(v)).collect()
    }

    /// Shrink an unsatisfiable `set` to a minimal unsatisfiable subset by
    /// deletion, refining with each core.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn shrink(&mut self, set: &BTreeSet<usize>, budget: &Budget) -> Result<BTreeSet<usize>, ComposeError> {
        let mut mus = set.clone();
        let order: Vec<usize> = set.iter().copied().collect();
        for e in order {
            budget.check_time()?;
            if !mus.contains(&e) {
                continue;
            }
            let mut without = mus.clone();
            without.remove(&e);
            if !self.check(&without) {
                let core = self.core();
                mus = without.intersection(&core).copied().collect();
                if self.check(&mus) {
                    mus = without; // the backend's core was not a core; keep the safe set
                }
            }
        }
        Ok(mus)
    }

    /// One MUS of the full set, if it is unsatisfiable.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn first_mus(&mut self, budget: &Budget) -> Result<Option<BTreeSet<usize>>, ComposeError> {
        let all: BTreeSet<usize> = (0..self.selectors.len()).collect();
        if self.check(&all) {
            return Ok(None);
        }
        let core = self.core();
        let start = if self.check(&core) { all } else { core };
        self.shrink(&start, budget).map(Some)
    }

    /// Up to `max` MUSes by MARCO (Liffiton et al.): a map solver over the
    /// selectors proposes unexplored seeds; SAT seeds grow to MSSes and are
    /// blocked from above, UNSAT seeds shrink to MUSes and are blocked from below.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn enumerate(&mut self, max: usize, budget: &Budget) -> Result<Vec<BTreeSet<usize>>, ComposeError> {
        self.enumerate_where(None, max, budget)
    }

    /// [`Self::enumerate`] restricted to the cores that contain (`(i, true)`)
    /// or avoid (`(i, false)`) element `i`; exact, since the restriction is a
    /// unit clause on the map solver.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn enumerate_where(
        &mut self,
        force: Option<(usize, bool)>,
        max: usize,
        budget: &Budget,
    ) -> Result<Vec<BTreeSet<usize>>, ComposeError> {
        let n = self.selectors.len();
        let mut map = Sat::new();
        let m: Vec<u32> = (0..n).map(|_| map.var()).collect();
        if let Some((i, want)) = force {
            map.add(&[(m[i], want)]);
        }
        let frozen = force.and_then(|(i, want)| (!want).then_some(i));
        let mut out = Vec::new();
        while out.len() < max {
            budget.check_time()?;
            if !map.solve(&[]) {
                break;
            }
            let seed: BTreeSet<usize> = (0..n).filter(|&i| map.value(m[i])).collect();
            if self.check(&seed) {
                let mut mss = seed;
                for i in 0..n {
                    if !mss.contains(&i) && frozen != Some(i) {
                        mss.insert(i);
                        if !self.check(&mss) {
                            mss.remove(&i);
                        }
                    }
                }
                let block: Vec<L> = (0..n).filter(|i| !mss.contains(i)).map(|i| (m[i], true)).collect();
                if block.is_empty() {
                    break;
                }
                map.add(&block);
            } else {
                let mus = self.shrink(&seed, budget)?;
                let block: Vec<L> = mus.iter().map(|&i| (m[i], false)).collect();
                map.add(&block);
                if force.is_none_or(|(i, want)| mus.contains(&i) == want) {
                    out.push(mus);
                }
            }
        }
        Ok(out)
    }
}

/// How a bundle's inconsistency relates to one claim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StagedCores {
    /// Some minimal unsat core contains the claim.
    pub involves_claim: bool,
    /// Minimal unsat cores (element ids) that do not contain the claim.
    pub peer_cores: Vec<Vec<String>>,
}

/// Classify the minimal unsat cores of `b` by membership of `claim`,
/// listing up to `max_peer` cores that avoid it. Exact: membership is a
/// restricted MARCO run, not a prefix of an enumeration.
///
/// # Errors
/// Budget exhausted.
pub fn staged_cores(b: &Bundle, claim: &str, assume: &str, max_peer: usize, budget: &Budget) -> Result<StagedCores, ComposeError> {
    let els = elements(b, assume);
    let mut inst = Instance::new(b.claims.len(), &els);
    let unit = els.iter().position(|e| e.id == claim);
    let involves_claim = match unit {
        Some(u) => !inst.enumerate_where(Some((u, true)), 1, budget)?.is_empty(),
        None => false,
    };
    let peers = inst.enumerate_where(unit.map(|u| (u, false)), max_peer, budget)?;
    let peer_cores = peers.iter().map(|s| s.iter().map(|&i| els[i].id.clone()).collect()).collect();
    Ok(StagedCores { involves_claim, peer_cores })
}

pub struct SatCore {
    spec: ComposerSpec,
}

impl Default for SatCore {
    fn default() -> Self {
        Self { spec: ComposerSpec::builtin(SPEC) }
    }
}

impl Composer for SatCore {
    fn spec(&self) -> &ComposerSpec {
        &self.spec
    }

    fn compose(&self, b: &Bundle, params: &Value, _q: &Query, budget: &Budget) -> Result<Output, ComposeError> {
        known_params(params, &self.spec)?;
        let assume = param_enum(params, "assume", &["all", "noticed"], "all")?;
        let max_cores = match params.get("max_cores") {
            None | Some(Value::Null) => budget.max_cores.unwrap_or(1),
            Some(v) => v
                .as_u64()
                .filter(|&x| x >= 1)
                .and_then(|x| usize::try_from(x).ok())
                .ok_or_else(|| ComposeError::new("E_PARAMS_INVALID", "max_cores must be an integer >= 1"))?,
        };
        let els = elements(b, assume);
        let mut inst = Instance::new(b.claims.len(), &els);
        budget.check_vars(inst.sat.num_vars() as usize)?;
        let ids = |s: &BTreeSet<usize>| -> Vec<String> { s.iter().map(|&i| els[i].id.clone()).collect() };
        let all: BTreeSet<usize> = (0..els.len()).collect();
        let mut out = Output::default();
        if inst.check(&all) {
            let model = inst.model();
            let (t, f): (Vec<_>, Vec<_>) = b.claims.iter().zip(&model).partition(|(_, &v)| v);
            let t: Vec<String> = t.into_iter().map(|(c, _)| c.id.clone()).collect();
            let f: Vec<String> = f.into_iter().map(|(c, _)| c.id.clone()).collect();
            out.outputs.insert("satisfiable".into(), json!(true));
            out.outputs.insert("model".into(), json!({"true": t, "false": f}));
            out.evidence.push(Evidence::new("model", t.clone(), Some(json!({"true": t, "false": f}))));
            return Ok(out);
        }
        let cores = if max_cores > 1 {
            inst.enumerate(max_cores, budget)?
        } else {
            inst.first_mus(budget)?.into_iter().collect()
        };
        let cores: Vec<Vec<String>> = cores.iter().map(ids).collect();
        out.failed_decision = true;
        out.outputs.insert("satisfiable".into(), json!(false));
        out.outputs.insert("core".into(), json!(cores.first().cloned().unwrap_or_default()));
        out.outputs.insert("cores".into(), json!(cores));
        out.evidence.extend(cores.into_iter().map(|c| Evidence::new("unsat_core", c, None)));
        Ok(out)
    }
}

#[cfg(test)]
mod staged_tests {
    use super::*;

    #[test]
    fn claim_membership_in_cores_is_exact() {
        // a excludes b (hard); c is unrelated.
        let b = Bundle::from_json(serde_json::json!({
            "schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"},
            "claims": [{"id": "a", "text": "a", "prior": 0.5}, {"id": "b", "text": "b", "prior": 0.5}, {"id": "c", "text": "c", "prior": 0.5}],
            "exhibits": [], "links": [],
            "constraints": [{"id": "k", "kind": "excludes", "claims": ["a", "b"], "mode": "hard"}]
        }))
        .unwrap();
        let bud = ComposerSpec::builtin(SPEC).default_budget().started();
        let c = staged_cores(&b, "c", "all", 4, &bud).unwrap();
        assert!(!c.involves_claim);
        assert_eq!(c.peer_cores, vec![vec!["a".to_owned(), "b".into(), "k".into()]]);
        let a = staged_cores(&b, "a", "all", 4, &bud).unwrap();
        assert!(a.involves_claim);
        assert!(a.peer_cores.is_empty());
    }
}
