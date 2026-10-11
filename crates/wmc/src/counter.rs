//! A component-caching DPLL weighted model counter (sharpSAT-style): unit
//! propagation, connected-component decomposition, and a cache keyed on the
//! residual component's clauses. Weights are arbitrary non-negative reals
//! per literal; a variable that drops out of every clause contributes
//! `w(v) + w(!v)`.
//!
//! Literals are DIMACS-style `i32`: `v + 1` or `-(v + 1)` for var `v`.

use condorcet_composer::{Budget, ComposeError};
use std::collections::{BTreeSet, HashMap};

pub type Clause = Vec<i32>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub decisions: u64,
    pub cache_hits: u64,
    pub cache_entries: usize,
}

pub struct Counter<'a> {
    pos: &'a [f64],
    neg: &'a [f64],
    cache: HashMap<Vec<Clause>, f64>,
    budget: &'a Budget,
    pub stats: Stats,
}

const fn var(l: i32) -> usize {
    (l.unsigned_abs() - 1) as usize
}

fn vars_of(cs: &[Clause]) -> BTreeSet<usize> {
    cs.iter().flatten().map(|&l| var(l)).collect()
}

/// Assign literal `l` true: drop satisfied clauses, remove `!l`.
/// `None` when a clause becomes empty.
fn condition(cs: &[Clause], l: i32) -> Option<Vec<Clause>> {
    let mut out = Vec::with_capacity(cs.len());
    for c in cs {
        if c.contains(&l) {
            continue;
        }
        let r: Clause = c.iter().copied().filter(|&x| x != -l).collect();
        if r.is_empty() {
            return None;
        }
        out.push(r);
    }
    Some(out)
}

/// Split into variable-connected components.
fn components(cs: Vec<Clause>) -> Vec<Vec<Clause>> {
    let n = cs.iter().flatten().map(|&l| var(l)).max().map_or(0, |m| m + 1);
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut y = x;
        while p[y] != r {
            let nx = p[y];
            p[y] = r;
            y = nx;
        }
        r
    }
    for c in &cs {
        let a = find(&mut parent, var(c[0]));
        for &l in &c[1..] {
            let b = find(&mut parent, var(l));
            parent[b] = a;
        }
    }
    let mut groups: HashMap<usize, Vec<Clause>> = HashMap::new();
    for c in cs {
        let r = find(&mut parent, var(c[0]));
        groups.entry(r).or_default().push(c);
    }
    let mut v: Vec<Vec<Clause>> = groups.into_values().collect();
    v.sort();
    v
}

fn normalize(mut cs: Vec<Clause>) -> Vec<Clause> {
    for c in &mut cs {
        c.sort_unstable();
        c.dedup();
    }
    cs.retain(|c| !c.iter().any(|&l| c.contains(&-l)));
    cs.sort();
    cs.dedup();
    cs
}

impl<'a> Counter<'a> {
    #[must_use]
    pub fn new(pos: &'a [f64], neg: &'a [f64], budget: &'a Budget) -> Self {
        Self { pos, neg, cache: HashMap::new(), budget, stats: Stats::default() }
    }

    fn w(&self, l: i32) -> f64 {
        if l > 0 {
            self.pos[var(l)]
        } else {
            self.neg[var(l)]
        }
    }

    fn free(&self, v: usize) -> f64 {
        self.pos[v] + self.neg[v]
    }

    /// Weighted model count of `cs` over all `n` variables.
    ///
    /// # Errors
    /// Budget exhausted.
    pub fn count_all(&mut self, cs: &[Clause], n: usize) -> Result<f64, ComposeError> {
        let cs = normalize(cs.to_vec());
        if cs.iter().any(Vec::is_empty) {
            return Ok(0.0);
        }
        let used = vars_of(&cs);
        let free: f64 = (0..n).filter(|v| !used.contains(v)).map(|v| self.free(v)).product();
        Ok(free * self.count(cs)?)
    }

    /// Count over exactly the variables of `cs`.
    fn count(&mut self, mut cs: Vec<Clause>) -> Result<f64, ComposeError> {
        let mut factor = 1.0;
        loop {
            if cs.is_empty() {
                return Ok(factor);
            }
            let Some(unit) = cs.iter().find(|c| c.len() == 1).map(|c| c[0]) else { break };
            let before = vars_of(&cs);
            let Some(next) = condition(&cs, unit) else { return Ok(0.0) };
            let after = vars_of(&next);
            factor *= self.w(unit);
            factor *= before.iter().filter(|&&v| v != var(unit) && !after.contains(&v)).map(|&v| self.free(v)).product::<f64>();
            cs = next;
        }
        let before = vars_of(&cs);
        let cs = normalize(cs);
        let after = vars_of(&cs);
        let mut total = factor * before.difference(&after).map(|&v| self.free(v)).product::<f64>();
        for comp in components(cs) {
            total *= self.component(comp)?;
            if total == 0.0 {
                return Ok(0.0);
            }
        }
        Ok(total)
    }

    fn component(&mut self, cs: Vec<Clause>) -> Result<f64, ComposeError> {
        if let Some(&v) = self.cache.get(&cs) {
            self.stats.cache_hits += 1;
            return Ok(v);
        }
        self.stats.decisions += 1;
        if self.stats.decisions % 1024 == 0 {
            self.budget.check_time()?;
        }
        let mut occ: HashMap<usize, usize> = HashMap::new();
        for &l in cs.iter().flatten() {
            *occ.entry(var(l)).or_default() += 1;
        }
        let v = occ.into_iter().max_by_key(|&(v, n)| (n, std::cmp::Reverse(v))).map_or(0, |(v, _)| v);
        let lit = i32::try_from(v + 1).unwrap_or(i32::MAX);
        let before = vars_of(&cs);
        let mut branch = |l: i32| -> Result<f64, ComposeError> {
            let wl = self.w(l);
            if wl == 0.0 {
                return Ok(0.0);
            }
            let Some(next) = condition(&cs, l) else { return Ok(0.0) };
            let after = vars_of(&next);
            let vanished: f64 =
                before.iter().filter(|&&u| u != v && !after.contains(&u)).map(|&u| self.free(u)).product();
            Ok(wl * vanished * self.count(next)?)
        };
        let r = branch(lit)? + branch(-lit)?;
        self.cache.insert(cs, r);
        self.stats.cache_entries = self.cache.len();
        Ok(r)
    }
}

/// Brute force over all `2^n` assignments (test oracle).
#[must_use]
pub fn brute_force(cs: &[Clause], pos: &[f64], neg: &[f64]) -> f64 {
    let n = pos.len();
    (0u64..1 << n)
        .filter(|m| cs.iter().all(|c| c.iter().any(|&l| ((m >> var(l)) & 1 == 1) == (l > 0))))
        .map(|m| (0..n).map(|v| if (m >> v) & 1 == 1 { pos[v] } else { neg[v] }).product::<f64>())
        .sum()
}

/// An upper bound on the primal graph's treewidth (min-degree elimination).
#[must_use]
pub fn treewidth_upper_bound(cs: &[Clause], n: usize) -> usize {
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for c in cs {
        for &a in c {
            for &b in c {
                if var(a) != var(b) {
                    adj[var(a)].insert(var(b));
                }
            }
        }
    }
    let mut alive: BTreeSet<usize> = (0..n).collect();
    let mut width = 0;
    while let Some(&v) = alive.iter().min_by_key(|&&v| adj[v].len()) {
        let nb: Vec<usize> = adj[v].iter().copied().collect();
        width = width.max(nb.len());
        for &a in &nb {
            adj[a].remove(&v);
            for &b in &nb {
                if a != b {
                    adj[a].insert(b);
                }
            }
        }
        alive.remove(&v);
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_counts() {
        let b = Budget::default();
        let pos = [0.3, 0.6, 0.5];
        let neg = [0.7, 0.4, 0.5];
        let mut c = Counter::new(&pos, &neg, &b);
        // (x0 | x1) & !x2 over three vars
        let cs = vec![vec![1, 2], vec![-3]];
        let got = c.count_all(&cs, 3).unwrap();
        assert!((got - (1.0 - 0.7 * 0.4) * 0.5).abs() < 1e-12);
        assert!((got - brute_force(&cs, &pos, &neg)).abs() < 1e-12);
        assert_eq!(c.count_all(&[vec![1], vec![-1]], 3).unwrap(), 0.0);
        assert!((c.count_all(&[], 3).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn treewidth_of_a_chain_and_a_clique() {
        assert_eq!(treewidth_upper_bound(&[vec![1, 2], vec![2, 3], vec![3, 4]], 4), 1);
        assert_eq!(treewidth_upper_bound(&[vec![1, 2, 3, 4]], 4), 3);
    }
}
