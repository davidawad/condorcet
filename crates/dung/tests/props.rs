//! dung semantics against brute-force enumeration of every subset of small
//! random frameworks (definitions from Dung 1995), plus the bundle mapping.

use condorcet_composer::{Budget, Composer, Query};
use condorcet_core::Bundle;
use condorcet_dung::{sceptical, Af, Dung, Label};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

type Ext = BTreeSet<usize>;

fn set(m: u32, n: usize) -> Ext {
    (0..n).filter(|i| (m >> i) & 1 == 1).collect()
}

fn attacks(af: &Af, s: &Ext, a: usize) -> bool {
    af.attackers[a].iter().any(|b| s.contains(b))
}

fn conflict_free(af: &Af, s: &Ext) -> bool {
    s.iter().all(|&a| !attacks(af, s, a))
}

fn defends(af: &Af, s: &Ext, a: usize) -> bool {
    af.attackers[a].iter().all(|&b| attacks(af, s, b))
}

fn admissible(af: &Af, s: &Ext) -> bool {
    conflict_free(af, s) && s.iter().all(|&a| defends(af, s, a))
}

fn complete(af: &Af, s: &Ext) -> bool {
    admissible(af, s) && (0..af.len()).all(|a| !defends(af, s, a) || s.contains(&a))
}

fn all_sets(af: &Af) -> Vec<Ext> {
    (0u32..1 << af.len()).map(|m| set(m, af.len())).collect()
}

fn brute_preferred(af: &Af) -> BTreeSet<Ext> {
    let adm: Vec<Ext> = all_sets(af).into_iter().filter(|s| admissible(af, s)).collect();
    adm.iter().filter(|s| !adm.iter().any(|t| t.len() > s.len() && s.is_subset(t))).cloned().collect()
}

fn brute_stable(af: &Af) -> BTreeSet<Ext> {
    all_sets(af)
        .into_iter()
        .filter(|s| conflict_free(af, s) && (0..af.len()).all(|a| s.contains(&a) || attacks(af, s, a)))
        .collect()
}

fn brute_grounded(af: &Af) -> Ext {
    let comp: Vec<Ext> = all_sets(af).into_iter().filter(|s| complete(af, s)).collect();
    comp.iter().find(|s| comp.iter().all(|t| s.is_subset(t))).cloned().expect("a least complete extension exists")
}

fn af_strategy() -> impl Strategy<Value = Af> {
    (1usize..=7).prop_flat_map(|n| {
        proptest::collection::vec((0..n, 0..n), 0..=(2 * n)).prop_map(move |att| Af::new(n, &att))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn semantics_match_brute_force(af in af_strategy()) {
        let b = Budget::default();
        let g: Ext = af.grounded().iter().enumerate().filter(|(_, l)| **l == Label::In).map(|(i, _)| i).collect();
        prop_assert_eq!(g, brute_grounded(&af));
        let pref: BTreeSet<Ext> = af.preferred(10_000, &b).unwrap().into_iter().collect();
        prop_assert_eq!(pref, brute_preferred(&af));
        let st: BTreeSet<Ext> = af.stable(10_000, &b).unwrap().into_iter().collect();
        prop_assert_eq!(st.clone(), brute_stable(&af));
        // Every stable extension is preferred; the grounded labelling is the
        // labelling of the grounded extension.
        let g_ext: Ext = brute_grounded(&af);
        prop_assert_eq!(af.labelling(&g_ext), af.grounded());
        for s in &st {
            prop_assert!(brute_preferred(&af).contains(s));
        }
        let exts: Vec<Ext> = st.into_iter().collect();
        let lab = sceptical(&af, &exts);
        for a in 0..af.len() {
            if lab[a] == Label::In {
                prop_assert!(exts.iter().all(|e| e.contains(&a)));
            }
        }
    }
}

fn run(b: &Bundle, sem: &str) -> serde_json::Map<String, Value> {
    Dung::default().compose(b, &json!({"semantics": sem}), &Query::default(), &Budget::default()).unwrap().outputs
}

#[test]
fn video_clock_mapping_and_labels() {
    let v: Value = serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap();
    let b = Bundle::from_json(v).unwrap();
    let g = run(&b, "grounded");
    let reading = "link:org:clx/ex-video>org:clx/fct-a1";
    // The clock defeater is unattacked: in; it knocks the video reading out.
    assert_eq!(g["label"]["org:clx/def-clock"], "in");
    assert_eq!(g["label"][reading], "out");
    assert_eq!(g["attackers"][reading], json!([reading, "org:clx/def-clock"]));
    // a1 and b2 exclude each other: undecided in grounded, two preferred extensions.
    assert_eq!(g["label"]["org:clx/fct-a1"], "undecided");
    let p = run(&b, "preferred");
    assert_eq!(p["extensions"].as_array().unwrap().len(), 2);
    assert_eq!(p["label"]["org:clx/fct-a1"], "undecided");
    assert_eq!(run(&b, "stable")["exists"], json!(true));
}

#[test]
fn odd_cycle_has_no_stable_extension() {
    let b = Bundle::from_json(json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"},
        "claims": [{"id": "a", "text": "a"}, {"id": "b", "text": "b"}, {"id": "c", "text": "c"}],
        "links": [
            {"id": "la", "kind": "contradict", "from": "a", "to": "b", "weight": {"unweighted": true}},
            {"id": "ua", "kind": "undercut", "from": "c", "to": "la", "strength": 1.0}
        ]})).unwrap();
    let g = run(&b, "grounded");
    assert_eq!((g["label"]["c"].as_str(), g["label"]["la"].as_str(), g["label"]["b"].as_str()), (Some("in"), Some("out"), Some("in")));
    let af = Af::new(3, &[(0, 1), (1, 2), (2, 0)]);
    assert!(af.stable(10, &Budget::default()).unwrap().is_empty());
    assert_eq!(af.preferred(10, &Budget::default()).unwrap(), vec![Ext::new()]);
}
