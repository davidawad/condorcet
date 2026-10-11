//! The Bundle: what a fact adapter hands the engine (schema
//! `condorcet/bundle/v1`). Pure data plus structural validation; the engine
//! learns no domain rule from it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const BUNDLE_SCHEMA: &str = "condorcet/bundle/v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterProvenance {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_hint: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub noticed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub about: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    pub claim: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exhibit {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specimen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    pub reading: Reading,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<String>,
}

/// The weight on a support or contradict link.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weight {
    /// r in (0.5, 1): the probability the link fires.
    Reliability(f64),
    /// Natural-log likelihood ratio, > 0.
    Llr(f64),
    /// Logical composers only.
    Unweighted(bool),
}

impl Weight {
    /// The firing probability of the reliability encoding; `Llr(x)` is
    /// `1 - e^-x`, so the odds multiply by `e^x`. `None` when unweighted.
    #[must_use]
    pub fn reliability(self) -> Option<f64> {
        match self {
            Self::Reliability(r) => Some(r),
            Self::Llr(x) => Some(1.0 - (-x).exp()),
            Self::Unweighted(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Support,
    Contradict,
    Undercut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub id: String,
    pub kind: LinkKind,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<Weight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// No two of the claims hold together (pairwise; same as `at_most_one`).
    Excludes,
    /// The first claim implies each of the others.
    Implies,
    /// All the claims have the same truth value.
    Equiv,
    ExactlyOne,
    AtMostOne,
    AtLeastOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Hard,
    /// Log-odds penalty for a violation.
    Soft(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub id: String,
    pub kind: ConstraintKind,
    pub claims: Vec<String>,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

impl Constraint {
    /// The constraint as CNF over claim indices given by `ix`: each inner
    /// vector is a clause of `(claim index, polarity)`.
    #[must_use]
    pub fn clauses(&self, ix: &dyn Fn(&str) -> usize) -> Vec<Vec<(usize, bool)>> {
        let c: Vec<usize> = self.claims.iter().map(|s| ix(s)).collect();
        let pairs_excl = || {
            let mut out = Vec::new();
            for i in 0..c.len() {
                for j in i + 1..c.len() {
                    out.push(vec![(c[i], false), (c[j], false)]);
                }
            }
            out
        };
        match self.kind {
            ConstraintKind::Excludes | ConstraintKind::AtMostOne => pairs_excl(),
            ConstraintKind::AtLeastOne => vec![c.iter().map(|&x| (x, true)).collect()],
            ConstraintKind::ExactlyOne => {
                let mut v = pairs_excl();
                v.push(c.iter().map(|&x| (x, true)).collect());
                v
            }
            ConstraintKind::Implies => c
                .iter()
                .skip(1)
                .map(|&x| vec![(c[0], false), (x, true)])
                .collect(),
            ConstraintKind::Equiv => c
                .windows(2)
                .flat_map(|w| [vec![(w[0], false), (w[1], true)], vec![(w[1], false), (w[0], true)]])
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cluster {
    pub key: String,
    pub rho: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub schema: String,
    pub adapter: AdapterProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub exhibits: Vec<Exhibit>,
    pub links: Vec<Link>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub clusters: Vec<Cluster>,
}

/// Why a bundle is unusable. `code` is stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BundleError {}

fn bad(message: String) -> BundleError {
    BundleError { code: "E_BUNDLE_INVALID", message }
}

fn prob(x: f64) -> bool {
    x.is_finite() && (0.0..=1.0).contains(&x)
}

impl Bundle {
    /// Parse a bundle from JSON and check it.
    ///
    /// # Errors
    /// `E_BUNDLE_PARSE` for JSON that is not a bundle, `E_BUNDLE_INVALID` for
    /// a structurally broken one.
    pub fn from_json(v: serde_json::Value) -> Result<Self, BundleError> {
        let b: Self = serde_json::from_value(v)
            .map_err(|e| BundleError { code: "E_BUNDLE_PARSE", message: e.to_string() })?;
        b.validate()?;
        Ok(b)
    }

    /// Index of each claim id.
    #[must_use]
    pub fn claim_index(&self) -> BTreeMap<&str, usize> {
        self.claims.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect()
    }

    /// Claims that are the source of some undercut link (defeaters).
    #[must_use]
    pub fn defeaters(&self) -> BTreeSet<&str> {
        self.links
            .iter()
            .filter(|l| l.kind == LinkKind::Undercut)
            .map(|l| l.from.as_str())
            .collect()
    }

    /// Structural checks the JSON schema cannot express: unique ids,
    /// endpoints that exist and have the right kind, numbers in range.
    ///
    /// # Errors
    /// `E_BUNDLE_INVALID` naming the first problem found.
    pub fn validate(&self) -> Result<(), BundleError> {
        if self.schema != BUNDLE_SCHEMA {
            return Err(bad(format!("schema must be {BUNDLE_SCHEMA}, got {}", self.schema)));
        }
        let mut seen = BTreeSet::new();
        let ids = self
            .claims
            .iter()
            .map(|c| &c.id)
            .chain(self.exhibits.iter().map(|e| &e.id))
            .chain(self.links.iter().map(|l| &l.id));
        for id in ids {
            if id.is_empty() || !seen.insert(id.as_str()) {
                return Err(bad(format!("duplicate or empty id {id:?}")));
            }
        }
        let claims = self.claim_index();
        let exhibits: BTreeSet<&str> = self.exhibits.iter().map(|e| e.id.as_str()).collect();
        let links: BTreeMap<&str, LinkKind> =
            self.links.iter().map(|l| (l.id.as_str(), l.kind)).collect();
        for c in &self.claims {
            for (name, p) in [("prior", c.prior), ("p_hint", c.p_hint)] {
                if p.is_some_and(|p| !prob(p)) {
                    return Err(bad(format!("claim {}: {name} out of [0,1]", c.id)));
                }
            }
        }
        for e in &self.exhibits {
            if !claims.contains_key(e.reading.claim.as_str()) {
                return Err(bad(format!("exhibit {}: reading.claim {} is not a claim", e.id, e.reading.claim)));
            }
        }
        for l in &self.links {
            self.check_link(l, &claims, &exhibits, &links)?;
        }
        for k in &self.constraints {
            if k.claims.is_empty() {
                return Err(bad(format!("constraint {}: no claims", k.id)));
            }
            if let Some(c) = k.claims.iter().find(|c| !claims.contains_key(c.as_str())) {
                return Err(bad(format!("constraint {}: {c} is not a claim", k.id)));
            }
            if let Mode::Soft(w) = k.mode {
                if !(w.is_finite() && w > 0.0) {
                    return Err(bad(format!("constraint {}: soft weight must be > 0", k.id)));
                }
            }
        }
        for g in &self.clusters {
            if !prob(g.rho) {
                return Err(bad(format!("cluster {}: rho out of [0,1]", g.key)));
            }
        }
        Ok(())
    }

    fn check_link(
        &self,
        l: &Link,
        claims: &BTreeMap<&str, usize>,
        exhibits: &BTreeSet<&str>,
        links: &BTreeMap<&str, LinkKind>,
    ) -> Result<(), BundleError> {
        let _ = self;
        match l.kind {
            LinkKind::Support | LinkKind::Contradict => {
                if !claims.contains_key(l.to.as_str()) {
                    return Err(bad(format!("link {}: to {} is not a claim", l.id, l.to)));
                }
                if !claims.contains_key(l.from.as_str()) && !exhibits.contains(l.from.as_str()) {
                    return Err(bad(format!("link {}: from {} is neither claim nor exhibit", l.id, l.from)));
                }
                if l.strength.is_some() {
                    return Err(bad(format!("link {}: strength is only for undercut", l.id)));
                }
                match l.weight {
                    None => return Err(bad(format!("link {}: missing weight", l.id))),
                    Some(Weight::Reliability(r)) if !(r > 0.5 && r < 1.0) => {
                        return Err(bad(format!("link {}: reliability must be in (0.5, 1)", l.id)))
                    }
                    Some(Weight::Llr(x)) if !(x.is_finite() && x > 0.0) => {
                        return Err(bad(format!("link {}: llr must be > 0", l.id)))
                    }
                    Some(Weight::Unweighted(false)) => {
                        return Err(bad(format!("link {}: unweighted must be true", l.id)))
                    }
                    Some(_) => {}
                }
            }
            LinkKind::Undercut => {
                if !claims.contains_key(l.from.as_str()) {
                    return Err(bad(format!("undercut {}: from {} is not a claim", l.id, l.from)));
                }
                match links.get(l.to.as_str()) {
                    Some(LinkKind::Support | LinkKind::Contradict) => {}
                    _ => return Err(bad(format!("undercut {}: to {} is not a support/contradict link", l.id, l.to))),
                }
                if l.weight.is_some() {
                    return Err(bad(format!("undercut {}: weight is only for support/contradict", l.id)));
                }
                if !l.strength.is_some_and(prob) {
                    return Err(bad(format!("undercut {}: strength must be in [0,1]", l.id)));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap()
    }

    #[test]
    fn example_parses_validates_and_round_trips() {
        let b = Bundle::from_json(example()).unwrap();
        assert_eq!(Bundle::from_json(serde_json::to_value(&b).unwrap()).unwrap(), b);
        assert_eq!(b.defeaters().into_iter().collect::<Vec<_>>(), vec!["org:clx/def-clock"]);
    }

    #[test]
    fn llr_converts_to_reliability() {
        let r = Weight::Llr(19f64.ln()).reliability().unwrap();
        assert!((r - 18.0 / 19.0).abs() < 1e-12);
    }

    #[test]
    fn rejects_dangling_and_misdirected_links() {
        let mut v = example();
        v["links"][0]["to"] = json!("nope");
        assert_eq!(Bundle::from_json(v).unwrap_err().code, "E_BUNDLE_INVALID");
        let mut v = example();
        v["links"][1]["to"] = json!("org:clx/fct-a1");
        assert!(Bundle::from_json(v).is_err());
        let mut v = example();
        v["claims"][0]["lr_plus"] = json!(19);
        assert_eq!(Bundle::from_json(v).unwrap_err().code, "E_BUNDLE_PARSE");
    }

    #[test]
    fn constraint_clauses() {
        let k = Constraint {
            id: "k".into(),
            kind: ConstraintKind::ExactlyOne,
            claims: vec!["a".into(), "b".into(), "c".into()],
            mode: Mode::Hard,
            basis: None,
        };
        let ix = |s: &str| (s.as_bytes()[0] - b'a') as usize;
        assert_eq!(k.clauses(&ix).len(), 4);
    }
}
