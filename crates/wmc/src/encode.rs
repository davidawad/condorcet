//! Encoding S: the source-reliability propositional encoding of a bundle.
//!
//! - one variable `X_c` per claim, weighted by its prior (a defeater with no
//!   evidence of its own uses its `p_hint`); a noticed claim is a unit clause;
//! - one `R_L` per support/contradict link, weight `r_L`; the clause
//!   `(F_L and R_L and not U_L) -> X_c` (contradict: `-> not X_c`), where
//!   `F_L` is the source claim's variable when the link comes from a claim
//!   (absent when it comes from an exhibit);
//! - `U_L <-> OR_d (D_d and S_d)` over the link's undercutters, `D_d` the
//!   defeater claim, `S_d` weight `strength`;
//! - a link in a cluster with correlation `rho > 0` gets
//!   `R_L <-> (T_L ? Q_L : P_L)`, `T_L` weight `rho`, `P_L` weight `r_L`, and
//!   `Q_L` the cluster's shared draw thresholded at `r_L` (a comonotone chain
//!   over the cluster's distinct reliabilities), so each marginal stays
//!   `r_L` and members with equal `r` share one draw exactly;
//! - hard constraints are clauses; a soft constraint with penalty `w`
//!   multiplies a violating world's weight by `e^-w` (an exact violation
//!   indicator).
//!
//! With one support link, prior `pi` and reliability `r`, the posterior is
//! `pi / (pi + (1 - pi)(1 - r))`.

use crate::counter::Clause;
use condorcet_composer::ComposeError;
use condorcet_core::{Bundle, LinkKind, Mode};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct Encoding {
    pub pos: Vec<f64>,
    pub neg: Vec<f64>,
    pub clauses: Vec<Clause>,
    /// `X_c` per claim, in bundle order.
    pub claim_var: Vec<usize>,
    /// Links skipped (unweighted, under `unweighted: ignore`).
    pub ignored: Vec<String>,
}

impl Encoding {
    fn var(&mut self, p: f64, q: f64) -> usize {
        self.pos.push(p);
        self.neg.push(q);
        self.pos.len() - 1
    }

    fn prob(&mut self, p: f64) -> usize {
        self.var(p, 1.0 - p)
    }

    fn aux(&mut self) -> usize {
        self.var(1.0, 1.0)
    }

    #[must_use]
    pub fn lit(v: usize, positive: bool) -> i32 {
        let l = i32::try_from(v + 1).unwrap_or(i32::MAX);
        if positive {
            l
        } else {
            -l
        }
    }

    fn clause(&mut self, lits: &[(usize, bool)]) {
        self.clauses.push(lits.iter().map(|&(v, p)| Self::lit(v, p)).collect());
    }

    /// `a <-> (b and c)`
    fn and(&mut self, a: usize, b: usize, c: usize) {
        self.clause(&[(a, false), (b, true)]);
        self.clause(&[(a, false), (c, true)]);
        self.clause(&[(a, true), (b, false), (c, false)]);
    }

    /// `r <-> (t ? q : p)`
    fn ite(&mut self, r: usize, t: usize, q: usize, p: usize) {
        self.clause(&[(t, false), (q, false), (r, true)]);
        self.clause(&[(t, false), (q, true), (r, false)]);
        self.clause(&[(t, true), (p, false), (r, true)]);
        self.clause(&[(t, true), (p, true), (r, false)]);
    }

    #[must_use]
    pub fn num_vars(&self) -> usize {
        self.pos.len()
    }
}

/// The cluster (key, rho) governing a link: the first of its exhibit's
/// cluster keys that the bundle declares with `rho > 0`.
fn link_cluster<'a>(b: &'a Bundle, from: &str) -> Option<(&'a str, f64)> {
    let ex = b.exhibits.iter().find(|e| e.id == from)?;
    ex.clusters.iter().find_map(|k| b.clusters.iter().find(|g| &g.key == k && g.rho > 0.0).map(|g| (g.key.as_str(), g.rho)))
}

/// Build encoding S. `unweighted_ignore`: skip unweighted links instead of
/// failing.
///
/// # Errors
/// `E_WEIGHT_UNSUPPORTED` for an unweighted link when not ignoring.
#[allow(clippy::too_many_lines)]
pub fn encode(b: &Bundle, base_rate: f64, unweighted_ignore: bool) -> Result<Encoding, ComposeError> {
    let mut e = Encoding::default();
    let ix = b.claim_index();
    let has_evidence: BTreeSet<&str> =
        b.links.iter().filter(|l| l.kind != LinkKind::Undercut).map(|l| l.to.as_str()).collect();
    let defeaters = b.defeaters();
    for c in &b.claims {
        let p = match (c.prior, c.p_hint) {
            (_, Some(h)) if defeaters.contains(c.id.as_str()) && !has_evidence.contains(c.id.as_str()) => h,
            (Some(p), _) => p,
            _ => base_rate,
        };
        let v = e.prob(p);
        e.claim_var.push(v);
        if c.noticed {
            e.clause(&[(v, true)]);
        }
    }
    // Reliability per evidential link.
    let mut rel: Vec<(usize, f64)> = Vec::new();
    for (i, l) in b.links.iter().enumerate() {
        if l.kind == LinkKind::Undercut {
            continue;
        }
        match l.weight.and_then(condorcet_core::Weight::reliability) {
            Some(r) => rel.push((i, r)),
            None if unweighted_ignore => e.ignored.push(l.id.clone()),
            None => {
                return Err(ComposeError::new(
                    "E_WEIGHT_UNSUPPORTED",
                    &format!("link {} is unweighted; wmc-exact needs reliability or llr (or params.unweighted = ignore)", l.id),
                ))
            }
        }
    }
    // Cluster chains: per cluster, distinct reliabilities ascending.
    let mut levels: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for &(i, r) in &rel {
        if let Some((k, _)) = link_cluster(b, &b.links[i].from) {
            levels.entry(k).or_default().push(r);
        }
    }
    let mut chain: BTreeMap<&str, Vec<(f64, usize)>> = BTreeMap::new();
    for (k, mut rs) in levels {
        rs.sort_by(f64::total_cmp);
        rs.dedup_by(|a, b| (*a - *b).abs() < 1e-15);
        let n = rs.len();
        let mut vars = vec![0usize; n];
        vars[n - 1] = e.prob(rs[n - 1]);
        for j in (0..n - 1).rev() {
            let bj = e.prob(rs[j] / rs[j + 1]);
            let a = e.aux();
            e.and(a, vars[j + 1], bj);
            vars[j] = a;
        }
        chain.insert(k, rs.into_iter().zip(vars).collect());
    }
    for (i, r) in rel {
        let l = &b.links[i];
        let rv = match link_cluster(b, &l.from) {
            Some((k, rho)) => {
                let q = chain[k].iter().find(|(x, _)| (x - r).abs() < 1e-15).map_or(0, |&(_, v)| v);
                let t = e.prob(rho);
                let p = e.prob(r);
                let rv = e.aux();
                e.ite(rv, t, q, p);
                rv
            }
            None => e.prob(r),
        };
        let undercuts: Vec<_> = b.links.iter().filter(|u| u.kind == LinkKind::Undercut && u.to == l.id).collect();
        let u = if undercuts.is_empty() {
            None
        } else {
            let u = e.aux();
            let mut any = vec![(u, false)];
            for d in undercuts {
                let s = e.prob(d.strength.unwrap_or(1.0));
                let a = e.aux();
                e.and(a, e.claim_var[ix[d.from.as_str()]], s);
                e.clause(&[(u, true), (a, false)]);
                any.push((a, true));
            }
            e.clause(&any);
            Some(u)
        };
        let x = e.claim_var[ix[l.to.as_str()]];
        let mut cl = vec![(rv, false), (x, l.kind == LinkKind::Support)];
        if let Some(u) = u {
            cl.push((u, true));
        }
        if let Some(&f) = ix.get(l.from.as_str()) {
            cl.push((e.claim_var[f], false));
        }
        e.clause(&cl);
    }
    for k in &b.constraints {
        let at = |s: &str| e.claim_var[ix[s]];
        let cls: Vec<Vec<(usize, bool)>> = k.clauses(&at);
        match k.mode {
            Mode::Hard => cls.iter().for_each(|c| e.clause(c)),
            Mode::Soft(w) => {
                // v <-> violated; n_i <-> clause i is false.
                let v = e.var((-w).exp(), 1.0);
                let mut any = vec![(v, false)];
                for c in &cls {
                    let n = e.aux();
                    for &(x, p) in c {
                        e.clause(&[(n, false), (x, !p)]);
                    }
                    let mut back: Vec<(usize, bool)> = c.clone();
                    back.push((n, true));
                    e.clause(&back);
                    e.clause(&[(v, true), (n, false)]);
                    any.push((n, true));
                }
                e.clause(&any);
            }
        }
    }
    Ok(e)
}
