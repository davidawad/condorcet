//! sat-core against brute-force enumeration: satisfiability, minimality of
//! every reported core, and (with enumeration) the exact set of MUSes.
//! Plus the lottery, preface and contradictory-pair fixtures.

use condorcet_composer::{Budget, Composer, Query};
use condorcet_core::Bundle;
use condorcet_sat::{elements, Element, SatCore};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn brute_sat(n: usize, els: &[&Element]) -> bool {
    (0u32..1 << n).any(|m| {
        els.iter().all(|e| e.clauses.iter().all(|c| c.iter().any(|&(v, p)| ((m >> v) & 1 == 1) == p)))
    })
}

fn subset<'a>(els: &'a [Element], mask: u32) -> Vec<&'a Element> {
    els.iter().enumerate().filter(|(i, _)| (mask >> i) & 1 == 1).map(|(_, e)| e).collect()
}

fn brute_muses(n: usize, els: &[Element]) -> BTreeSet<BTreeSet<String>> {
    let unsat: Vec<u32> = (0u32..1 << els.len()).filter(|&m| !brute_sat(n, &subset(els, m))).collect();
    unsat
        .iter()
        .filter(|&&m| (0..els.len()).filter(|i| (m >> i) & 1 == 1).all(|i| brute_sat(n, &subset(els, m & !(1 << i)))))
        .map(|&m| subset(els, m).iter().map(|e| e.id.clone()).collect())
        .collect()
}

const KINDS: [&str; 6] = ["excludes", "implies", "equiv", "exactly_one", "at_most_one", "at_least_one"];

fn bundle_strategy() -> impl Strategy<Value = Value> {
    (1usize..=4).prop_flat_map(|n| {
        let claims = proptest::collection::vec(any::<bool>(), n);
        let cons = proptest::collection::vec((0usize..6, proptest::collection::btree_set(0..n, 1..=n.min(3))), 0..=4);
        let contra = proptest::collection::vec((0..n, 0..n), 0..=2);
        (Just(n), claims, cons, contra)
    })
    .prop_map(|(n, noticed, cons, contra)| {
        let claims: Vec<Value> =
            (0..n).map(|i| json!({"id": format!("c{i}"), "text": "t", "noticed": noticed[i]})).collect();
        let constraints: Vec<Value> = cons
            .iter()
            .enumerate()
            .map(|(k, (kind, cs))| json!({"id": format!("k{k}"), "kind": KINDS[*kind], "claims": cs.iter().map(|i| format!("c{i}")).collect::<Vec<_>>(), "mode": "hard"}))
            .collect();
        let links: Vec<Value> = contra
            .iter()
            .filter(|(a, b)| a != b)
            .enumerate()
            .map(|(k, (a, b))| json!({"id": format!("l{k}"), "kind": "contradict", "from": format!("c{a}"), "to": format!("c{b}"), "weight": {"unweighted": true}}))
            .collect();
        json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"}, "claims": claims, "links": links, "constraints": constraints})
    })
}

fn compose(b: &Bundle, params: Value) -> serde_json::Map<String, Value> {
    SatCore::default().compose(b, &params, &Query::default(), &Budget::default()).unwrap().outputs
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn agrees_with_brute_force(v in bundle_strategy(), assume_all in any::<bool>()) {
        let b = Bundle::from_json(v).unwrap();
        let assume = if assume_all { "all" } else { "noticed" };
        let els = elements(&b, assume);
        let all: Vec<&Element> = els.iter().collect();
        let sat = brute_sat(b.claims.len(), &all);
        let out = compose(&b, json!({"assume": assume}));
        prop_assert_eq!(out["satisfiable"].as_bool(), Some(sat));
        if sat {
            let t: BTreeSet<&str> = out["model"]["true"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            let ix = b.claim_index();
            let model = |c: usize| t.contains(b.claims[c].id.as_str());
            prop_assert!(els.iter().all(|e| e.clauses.iter().all(|c| c.iter().any(|&(v, p)| model(v) == p))));
            let _ = ix;
        } else {
            let core: BTreeSet<String> = out["core"].as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
            let muses = brute_muses(b.claims.len(), &els);
            prop_assert!(muses.contains(&core), "core {:?} is not a MUS of {:?}", core, muses);
            let enumerated = compose(&b, json!({"assume": assume, "max_cores": 10_000}));
            let got: BTreeSet<BTreeSet<String>> = enumerated["cores"].as_array().unwrap().iter()
                .map(|c| c.as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).collect();
            prop_assert_eq!(got, muses);
        }
    }
}

/// Lottery and preface share a shape: n believed claims `c_i`, each
/// exclusive with its negation-witness `w_i`, and a constraint that at
/// least one witness holds. Every claim is needed for the contradiction.
fn lottery(n: usize) -> Bundle {
    let mut claims = vec![];
    let mut constraints = vec![];
    for i in 0..n {
        claims.push(json!({"id": format!("lose{i}"), "text": format!("ticket {i} loses")}));
        claims.push(json!({"id": format!("win{i}"), "text": format!("ticket {i} wins"), "prior": 0.1}));
        constraints.push(json!({"id": format!("x{i}"), "kind": "excludes", "claims": [format!("lose{i}"), format!("win{i}")], "mode": "hard"}));
    }
    constraints.push(json!({"id": "some-winner", "kind": "at_least_one", "claims": (0..n).map(|i| format!("win{i}")).collect::<Vec<_>>(), "mode": "hard"}));
    let v = json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"}, "claims": claims, "links": [], "constraints": constraints});
    Bundle::from_json(v).unwrap()
}

#[test]
fn lottery_is_unsat_and_its_core_needs_every_ticket() {
    let mut b = lottery(10);
    // Only the "loses" beliefs are asserted; the witnesses are free.
    for c in &mut b.claims {
        c.noticed = c.id.starts_with("lose");
    }
    let out = compose(&b, json!({"assume": "noticed"}));
    assert_eq!(out["satisfiable"], json!(false));
    assert_eq!(out["core"].as_array().unwrap().len(), 21);
    // Drop one belief: consistent.
    b.claims[0].noticed = false;
    assert_eq!(compose(&b, json!({"assume": "noticed"}))["satisfiable"], json!(true));
}

#[test]
fn contradictory_pair_core_is_the_pair_and_the_constraint() {
    let v: Value = serde_json::from_str(include_str!("../../../fixtures/contradictory-pair.json")).unwrap();
    let b = Bundle::from_json(v).unwrap();
    let out = compose(&b, json!({}));
    assert_eq!(out["satisfiable"], json!(false));
    assert_eq!(out["core"], json!(["org:t/a", "org:t/b", "k-ab"]));
}

#[test]
fn video_clock_is_consistent_and_defeater_is_not_asserted() {
    let v: Value = serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap();
    let b = Bundle::from_json(v).unwrap();
    // a1 and b2 exclude each other and both are asserted: unsat.
    let out = compose(&b, json!({}));
    assert_eq!(out["satisfiable"], json!(false));
    assert_eq!(out["core"], json!(["org:clx/fct-a1", "org:clx/fct-b2", "c1"]));
    assert_eq!(compose(&b, json!({"assume": "noticed"}))["satisfiable"], json!(true));
}
