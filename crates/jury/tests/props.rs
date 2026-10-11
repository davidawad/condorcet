//! condorcet-jury against brute force: the DP tail equals enumeration over
//! every pattern of correct votes; homogeneous juries match the closed
//! form; the theorem's monotonicity holds; clusters collapse.

use condorcet_composer::{Budget, Composer, Query};
use condorcet_core::Bundle;
use condorcet_jury::{majority_correct, Jury};
use proptest::prelude::*;
use serde_json::json;

fn brute(p: &[f64]) -> f64 {
    let n = p.len();
    (0u32..1 << n)
        .map(|m| {
            let k = m.count_ones() as usize;
            let w: f64 = (0..n).map(|i| if (m >> i) & 1 == 1 { p[i] } else { 1.0 - p[i] }).product();
            match (2 * k).cmp(&n) {
                std::cmp::Ordering::Greater => w,
                std::cmp::Ordering::Equal => w / 2.0,
                std::cmp::Ordering::Less => 0.0,
            }
        })
        .sum()
}

fn binom(n: u64, k: u64) -> f64 {
    (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn dp_matches_enumeration(p in proptest::collection::vec(0.0f64..=1.0, 0..=12)) {
        prop_assert!((majority_correct(&p) - brute(&p)).abs() < 1e-12);
    }

    #[test]
    fn homogeneous_closed_form_and_monotone(p in 0.51f64..0.99, half in 0u64..8) {
        let n = 2 * half + 1;
        let closed: f64 = (n.div_ceil(2)..=n).map(|k| binom(n, k) * p.powi(k as i32) * (1.0 - p).powi((n - k) as i32)).sum();
        let pn = majority_correct(&vec![p; n as usize]);
        prop_assert!((pn - closed).abs() < 1e-10);
        prop_assert!(majority_correct(&vec![p; n as usize + 2]) > pn);
        prop_assert!((majority_correct(&vec![0.5; n as usize]) - 0.5).abs() < 1e-12);
    }
}

#[test]
fn correlated_copies_collapse_to_one_juror_with_ceiling() {
    let ex = |i: usize| json!({"id": format!("e{i}"), "reading": {"claim": "c"}, "clusters": ["wire"]});
    let link = |i: usize| json!({"id": format!("l{i}"), "kind": "support", "from": format!("e{i}"), "to": "c", "weight": {"reliability": 0.6}});
    let b = Bundle::from_json(json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"},
        "claims": [{"id": "c", "text": "story"}], "exhibits": (0..5).map(ex).collect::<Vec<_>>(),
        "links": (0..5).map(link).collect::<Vec<_>>(), "clusters": [{"key": "wire", "rho": 0.3}]})).unwrap();
    let o = Jury::default().compose(&b, &json!({}), &Query::default(), &Budget::default()).unwrap().outputs;
    assert!((o["majority_correct"]["c"].as_f64().unwrap() - 0.6).abs() < 1e-12);
    assert!((o["raw_majority_correct"]["c"].as_f64().unwrap() - 0.682_56).abs() < 1e-12);
    assert!((o["ceiling"]["c"].as_f64().unwrap() - 0.88).abs() < 1e-12);
    assert_eq!(o["majority"]["c"], "true");
    let o = Jury::default().compose(&b, &json!({"clusters": "ignore"}), &Query::default(), &Budget::default()).unwrap().outputs;
    assert!((o["majority_correct"]["c"].as_f64().unwrap() - 0.682_56).abs() < 1e-12);
}
