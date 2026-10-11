//! Composer `dung@1`: abstract argumentation (Dung 1995) over a bundle.
//!
//! Arguments are the claims and the evidential readings (support and
//! contradict links). Attacks: an undercut link makes its defeater claim
//! attack the reading; a contradict reading attacks the claim it targets;
//! claims in an `excludes`, `at_most_one` or `exactly_one` constraint attack
//! each other symmetrically.
//!
//! Grounded semantics by fixpoint; complete labellings as SAT models
//! (Caminada labelling encoding); stable = complete with nothing
//! undecided; preferred = maximal complete extensions by iterated
//! superset search with subset blocking.

use condorcet_composer::{known_params, param_enum, Budget, ComposeError, Composer, ComposerSpec, Output, Query};
use condorcet_core::{Bundle, ConstraintKind, Evidence, LinkKind};
use condorcet_sat::solver::{Sat, L};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const SPEC: &str = include_str!("../../../composers/dung/composer.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    In,
    Out,
    Undecided,
}

impl Label {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Undecided => "undecided",
        }
    }
}

/// An argumentation framework: `attackers[a]` are the arguments attacking `a`.
#[derive(Debug, Clone, Default)]
pub struct Af {
    pub args: Vec<String>,
    pub attackers: Vec<BTreeSet<usize>>,
}

impl Af {
    #[must_use]
    pub fn new(n: usize, attacks: &[(usize, usize)]) -> Self {
        let mut attackers = vec![BTreeSet::new(); n];
        for &(a, b) in attacks {
            attackers[b].insert(a);
        }
        Self { args: (0..n).map(|i| i.to_string()).collect(), attackers }
    }

    #[must_use]
    pub fn from_bundle(b: &Bundle) -> Self {
        let mut args: Vec<String> = b.claims.iter().map(|c| c.id.clone()).collect();
        args.extend(b.links.iter().filter(|l| l.kind != LinkKind::Undercut).map(|l| l.id.clone()));
        let at = |id: &str| args.iter().position(|a| a == id);
        let mut attacks = Vec::new();
        for l in &b.links {
            match l.kind {
                LinkKind::Undercut => attacks.extend(at(&l.from).zip(at(&l.to))),
                LinkKind::Contradict => attacks.extend(at(&l.id).zip(at(&l.to))),
                LinkKind::Support => {}
            }
        }
        for k in &b.constraints {
            if matches!(k.kind, ConstraintKind::Excludes | ConstraintKind::AtMostOne | ConstraintKind::ExactlyOne) {
                for x in &k.claims {
                    for y in &k.claims {
                        if x != y {
                            attacks.extend(at(x).zip(at(y)));
                        }
                    }
                }
            }
        }
        let mut af = Self::new(args.len(), &attacks);
        af.args = args;
        af
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.args.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    /// The grounded labelling: least fixpoint of the characteristic function.
    #[must_use]
    pub fn grounded(&self) -> Vec<Label> {
        let mut lab = vec![Label::Undecided; self.len()];
        loop {
            let mut changed = false;
            for a in 0..self.len() {
                if lab[a] != Label::Undecided {
                    continue;
                }
                if self.attackers[a].iter().all(|&b| lab[b] == Label::Out) {
                    lab[a] = Label::In;
                    changed = true;
                } else if self.attackers[a].iter().any(|&b| lab[b] == Label::In) {
                    lab[a] = Label::Out;
                    changed = true;
                }
            }
            if !changed {
                return lab;
            }
        }
    }

    /// Labelling of an extension: in, out when attacked by it, else undecided.
    #[must_use]
    pub fn labelling(&self, ext: &BTreeSet<usize>) -> Vec<Label> {
        (0..self.len())
            .map(|a| {
                if ext.contains(&a) {
                    Label::In
                } else if self.attackers[a].iter().any(|b| ext.contains(b)) {
                    Label::Out
                } else {
                    Label::Undecided
                }
            })
            .collect()
    }

    /// SAT encoding of complete labellings; vars `in_a = 2a`, `out_a = 2a+1`.
    fn complete_encoding(&self, stable: bool) -> Sat {
        let mut s = Sat::new();
        let n = u32::try_from(self.len()).unwrap_or(u32::MAX);
        let (i, o) = (|a: usize| 2 * a as u32, |a: usize| 2 * a as u32 + 1);
        for _ in 0..2 * n {
            s.var();
        }
        for a in 0..self.len() {
            s.add(&[(i(a), false), (o(a), false)]);
            let mut all_out: Vec<L> = vec![(i(a), true)];
            let mut some_in: Vec<L> = vec![(o(a), false)];
            for &b in &self.attackers[a] {
                s.add(&[(i(a), false), (o(b), true)]);
                s.add(&[(o(a), true), (i(b), false)]);
                all_out.push((o(b), false));
                some_in.push((i(b), true));
            }
            s.add(&all_out);
            s.add(&some_in);
            if stable {
                s.add(&[(i(a), true), (o(a), true)]);
            }
        }
        s
    }

    fn in_set(&self, s: &Sat) -> BTreeSet<usize> {
        (0..self.len()).filter(|&a| s.value(2 * a as u32)).collect()
    }

    /// All stable extensions, up to `max`.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn stable(&self, max: usize, budget: &Budget) -> Result<Vec<BTreeSet<usize>>, ComposeError> {
        let mut s = self.complete_encoding(true);
        let mut out = Vec::new();
        while out.len() < max && s.solve(&[]) {
            budget.check_time()?;
            let e = self.in_set(&s);
            let block: Vec<L> = (0..self.len()).map(|a| (2 * a as u32, !e.contains(&a))).collect();
            out.push(e);
            if block.is_empty() {
                break;
            }
            s.add(&block);
        }
        Ok(out)
    }

    /// All preferred extensions (maximal complete), up to `max`.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn preferred(&self, max: usize, budget: &Budget) -> Result<Vec<BTreeSet<usize>>, ComposeError> {
        let mut s = self.complete_encoding(false);
        let mut out = Vec::new();
        while out.len() < max && s.solve(&[]) {
            let mut e = self.in_set(&s);
            loop {
                budget.check_time()?;
                let outside: Vec<usize> = (0..self.len()).filter(|a| !e.contains(a)).collect();
                if outside.is_empty() {
                    break;
                }
                let act = s.var();
                let mut grow: Vec<L> = vec![(act, false)];
                grow.extend(outside.iter().map(|&a| (2 * a as u32, true)));
                s.add(&grow);
                let mut assumps: Vec<L> = vec![(act, true)];
                assumps.extend(e.iter().map(|&a| (2 * a as u32, true)));
                let bigger = s.solve(&assumps);
                if bigger {
                    e = self.in_set(&s);
                }
                s.add(&[(act, false)]);
                if !bigger {
                    break;
                }
            }
            let block: Vec<L> = (0..self.len()).filter(|a| !e.contains(a)).map(|a| (2 * a as u32, true)).collect();
            out.push(e);
            if block.is_empty() {
                break;
            }
            s.add(&block);
        }
        Ok(out)
    }

    /// Why `a` is out under `lab`: `a`, an in attacker, one of its (out)
    /// attackers, that one's in attacker, ... (bounded, no repeats).
    #[must_use]
    pub fn chain(&self, a: usize, lab: &[Label]) -> Vec<String> {
        let mut path = vec![a];
        let mut cur = a;
        while path.len() < 12 {
            let want = if lab[cur] == Label::Out { Label::In } else { Label::Out };
            let Some(&next) = self.attackers[cur].iter().find(|&&b| lab[b] == want && !path.contains(&b)) else { break };
            path.push(next);
            cur = next;
        }
        path.into_iter().map(|i| self.args[i].clone()).collect()
    }
}

/// Sceptical labelling over a set of extensions: in if in all, out if out
/// in all, else undecided (all undecided when there are none).
#[must_use]
pub fn sceptical(af: &Af, exts: &[BTreeSet<usize>]) -> Vec<Label> {
    let labs: Vec<Vec<Label>> = exts.iter().map(|e| af.labelling(e)).collect();
    (0..af.len())
        .map(|a| match labs.first().map(|l| l[a]) {
            Some(first) if labs.iter().all(|l| l[a] == first) && first != Label::Undecided => first,
            _ => Label::Undecided,
        })
        .collect()
}

pub struct Dung {
    spec: ComposerSpec,
}

impl Default for Dung {
    fn default() -> Self {
        Self { spec: ComposerSpec::builtin(SPEC) }
    }
}

impl Composer for Dung {
    fn spec(&self) -> &ComposerSpec {
        &self.spec
    }

    fn compose(&self, b: &Bundle, params: &Value, _q: &Query, budget: &Budget) -> Result<Output, ComposeError> {
        known_params(params, &self.spec)?;
        let sem = param_enum(params, "semantics", &["grounded", "preferred", "stable"], "grounded")?;
        let max = params
            .get("max_extensions")
            .and_then(Value::as_u64)
            .and_then(|x| usize::try_from(x).ok())
            .unwrap_or(64)
            .max(1);
        let af = Af::from_bundle(b);
        budget.check_vars(2 * af.len())?;
        let (labels, exts, explain_lab) = if sem == "grounded" {
            let g = af.grounded();
            let e: BTreeSet<usize> = (0..af.len()).filter(|&a| g[a] == Label::In).collect();
            (g.clone(), vec![e], g)
        } else {
            let exts = if sem == "stable" { af.stable(max, budget)? } else { af.preferred(max, budget)? };
            let lab = sceptical(&af, &exts);
            let first = exts.first().map_or_else(|| lab.clone(), |e| af.labelling(e));
            (lab, exts, first)
        };
        let mut label = Map::new();
        let mut chains = Map::new();
        for (a, id) in af.args.iter().enumerate() {
            label.insert(id.clone(), json!(labels[a].as_str()));
            if labels[a] == Label::Out {
                chains.insert(id.clone(), json!(af.chain(a, &explain_lab)));
            }
        }
        let names = |e: &BTreeSet<usize>| -> Vec<String> { e.iter().map(|&a| af.args[a].clone()).collect() };
        let ext_ids: Vec<Vec<String>> = exts.iter().map(names).collect();
        let credulous: BTreeSet<String> = ext_ids.iter().flatten().cloned().collect();
        let mut out = Output::default();
        out.outputs.insert("semantics".into(), json!(sem));
        out.outputs.insert("label".into(), Value::Object(label));
        out.outputs.insert("extensions".into(), json!(ext_ids));
        out.outputs.insert("exists".into(), json!(!exts.is_empty()));
        out.outputs.insert("credulous".into(), json!(credulous));
        out.outputs.insert("attackers".into(), Value::Object(chains.clone()));
        out.evidence.extend(ext_ids.into_iter().map(|e| Evidence::new("extension", e, Some(json!(sem)))));
        if !chains.is_empty() {
            out.evidence.push(Evidence::new("note", chains.keys().cloned().collect(), Some(json!({"attacker_chains": chains}))));
        }
        Ok(out)
    }
}
