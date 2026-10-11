//! A small incremental SAT handle over rustsat's BatSat backend (pure Rust,
//! no C++ build). Literals are `(var, positive)`; vars count from 0.

use rustsat::solvers::{Solve, SolveIncremental, SolverResult};
use rustsat::types::{Clause, Lit, TernaryVal};
use rustsat_batsat::BasicSolver;
use std::collections::BTreeSet;

pub type L = (u32, bool);

fn lit((v, pos): L) -> Lit {
    Lit::new(v, !pos)
}

#[derive(Default)]
pub struct Sat {
    solver: BasicSolver,
    vars: u32,
}

impl Sat {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn var(&mut self) -> u32 {
        let v = self.vars;
        self.vars += 1;
        v
    }

    #[must_use]
    pub const fn num_vars(&self) -> u32 {
        self.vars
    }

    /// # Panics
    /// When the backend rejects a clause (out of memory).
    pub fn add(&mut self, clause: &[L]) {
        if let Some(max) = clause.iter().map(|l| l.0).max() {
            self.vars = self.vars.max(max + 1);
            let _ = self.solver.reserve(rustsat::types::Var::new(max));
        }
        let c: Clause = clause.iter().copied().map(lit).collect();
        self.solver.add_clause(c).expect("clause accepted");
    }

    /// `true` when satisfiable under `assumps`.
    ///
    /// # Panics
    /// When the backend reports neither SAT nor UNSAT (it is not interrupted here).
    pub fn solve(&mut self, assumps: &[L]) -> bool {
        if self.vars > 0 {
            let _ = self.solver.reserve(rustsat::types::Var::new(self.vars - 1));
        }
        let a: Vec<Lit> = assumps.iter().copied().map(lit).collect();
        match self.solver.solve_assumps(&a).expect("solver runs") {
            SolverResult::Sat => true,
            SolverResult::Unsat => false,
            SolverResult::Interrupted => panic!("solver interrupted"),
        }
    }

    /// The value of `v` in the last model (unassigned reads as false).
    #[must_use]
    pub fn value(&self, v: u32) -> bool {
        matches!(self.solver.lit_val(Lit::new(v, false)), Ok(TernaryVal::True))
    }

    /// The variables of the assumptions in the last UNSAT core.
    #[must_use]
    pub fn core_vars(&mut self) -> BTreeSet<u32> {
        self.solver.core().unwrap_or_default().into_iter().map(|l| l.var().idx32()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_and_reports_cores() {
        let mut s = Sat::new();
        let (a, b, sa, sb) = (s.var(), s.var(), s.var(), s.var());
        s.add(&[(sa, false), (a, true)]);
        s.add(&[(sb, false), (a, false)]);
        s.add(&[(b, true)]);
        assert!(s.solve(&[(sa, true)]));
        assert!(s.value(a) && s.value(b));
        assert!(!s.solve(&[(sa, true), (sb, true)]));
        assert_eq!(s.core_vars(), [sa, sb].into_iter().collect());
        assert!(s.solve(&[(sb, true)]));
        assert!(!s.value(a));
    }
}
