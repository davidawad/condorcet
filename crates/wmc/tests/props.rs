//! wmc-exact proven against two independent oracles:
//! 1. the counter against brute-force enumeration of random weighted CNFs;
//! 2. the composer's marginals and joints against a direct enumeration of
//!    the generative model the encoding claims to implement (claim truth,
//!    link firings, cluster copy/own draws with a discretised shared draw,
//!    undercut strengths, hard and soft constraints), written from the
//!    model's description, not from the encoding.

use condorcet_composer::{Budget, Composer, Query};
use condorcet_core::{Bundle, LinkKind, Mode};
use condorcet_wmc::counter::{brute_force, Counter};
use condorcet_wmc::WmcExact;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn cnf() -> impl Strategy<Value = (usize, Vec<Vec<i32>>, Vec<f64>, Vec<f64>)> {
    (1usize..=10).prop_flat_map(|n| {
        let n32 = i32::try_from(n).unwrap();
        let lit = (1..=n32, any::<bool>()).prop_map(|(v, s)| if s { v } else { -v });
        (
            Just(n),
            proptest::collection::vec(proptest::collection::vec(lit, 1..=3), 0..=12),
            proptest::collection::vec(0.0f64..2.0, n),
            proptest::collection::vec(0.0f64..2.0, n),
        )
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn counter_matches_brute_force((n, cs, pos, neg) in cnf()) {
        let b = Budget::default();
        let mut c = Counter::new(&pos, &neg, &b);
        let got = c.count_all(&cs, n).unwrap();
        let want = brute_force(&cs, &pos, &neg);
        prop_assert!(close(got, want), "{got} vs {want}");
    }
}

// ---------------------------------------------------------------- oracle

fn prior_of(b: &Bundle, i: usize, base: f64) -> f64 {
    let c = &b.claims[i];
    let defeater = b.defeaters().contains(c.id.as_str());
    let evidenced = b.links.iter().any(|l| l.kind != LinkKind::Undercut && l.to == c.id);
    match (c.prior, c.p_hint) {
        (_, Some(h)) if defeater && !evidenced => h,
        (Some(p), _) => p,
        _ => base,
    }
}

fn cluster_of(b: &Bundle, from: &str) -> Option<(String, f64)> {
    let ex = b.exhibits.iter().find(|e| e.id == from)?;
    ex.clusters.iter().find_map(|k| b.clusters.iter().find(|g| &g.key == k && g.rho > 0.0).map(|g| (g.key.clone(), g.rho)))
}

/// Unnormalised weight of every claim assignment (bitmask), summed over
/// all latent draws.
fn oracle(b: &Bundle, base: f64) -> BTreeMap<u32, f64> {
    let n = b.claims.len();
    let ix = b.claim_index();
    let ev: Vec<(usize, f64, Option<(String, f64)>)> = b
        .links
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind != LinkKind::Undercut)
        .map(|(i, l)| (i, l.weight.unwrap().reliability().unwrap(), cluster_of(b, &l.from)))
        .collect();
    let mut levels: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (_, r, g) in &ev {
        if let Some((k, _)) = g {
            levels.entry(k.clone()).or_default().push(*r);
        }
    }
    for v in levels.values_mut() {
        v.sort_by(f64::total_cmp);
        v.dedup();
    }
    let unders: Vec<usize> = (0..b.links.len()).filter(|&i| b.links[i].kind == LinkKind::Undercut).collect();
    // Dimensions: per evidential link (copy?, own fire?), per cluster level, per undercut.
    let mut dims: Vec<usize> = vec![];
    for _ in &ev {
        dims.extend([2, 2]);
    }
    let keys: Vec<String> = levels.keys().cloned().collect();
    for k in &keys {
        dims.push(levels[k].len() + 1);
    }
    dims.extend(unders.iter().map(|_| 2));
    let total: usize = dims.iter().product();
    let mut out = BTreeMap::new();
    for m in 0u32..1 << n {
        let x = |i: usize| (m >> i) & 1 == 1;
        let mut w: f64 = (0..n).map(|i| if x(i) { prior_of(b, i, base) } else { 1.0 - prior_of(b, i, base) }).product();
        if b.claims.iter().enumerate().any(|(i, c)| c.noticed && !x(i)) {
            continue;
        }
        for k in &b.constraints {
            let ok = k.clauses(&|s: &str| ix[s]).iter().all(|c| c.iter().any(|&(v, p)| x(v) == p));
            match k.mode {
                Mode::Hard if !ok => w = 0.0,
                Mode::Soft(pen) if !ok => w *= (-pen).exp(),
                _ => {}
            }
        }
        if w == 0.0 {
            continue;
        }
        let mut sum = 0.0;
        for mut d in 0..total {
            let mut digit = |base: usize| {
                let r = d % base;
                d /= base;
                r
            };
            let link_draws: Vec<(usize, usize)> = ev.iter().map(|_| (digit(2), digit(2))).collect();
            let u: BTreeMap<&String, usize> = keys.iter().map(|k| (k, digit(levels[k].len() + 1))).collect();
            let s: Vec<usize> = unders.iter().map(|_| digit(2)).collect();
            let mut p = 1.0;
            let mut fires = vec![false; b.links.len()];
            for ((li, r, g), (copy, own)) in ev.iter().zip(&link_draws) {
                match g {
                    Some((k, rho)) => {
                        p *= if *copy == 1 { *rho } else { 1.0 - rho };
                        if *copy == 1 {
                            let lv = levels[k].iter().position(|x| x == r).unwrap();
                            fires[*li] = u[k] <= lv;
                            p *= if *own == 0 { 1.0 } else { 0.0 }; // own draw unused: count once
                        } else {
                            p *= if *own == 1 { *r } else { 1.0 - r };
                            fires[*li] = *own == 1;
                        }
                    }
                    None => {
                        p *= if *copy == 0 { 1.0 } else { 0.0 };
                        p *= if *own == 1 { *r } else { 1.0 - r };
                        fires[*li] = *own == 1;
                    }
                }
            }
            for k in &keys {
                let lv = &levels[k];
                let lo = if u[k] == 0 { 0.0 } else { lv[u[k] - 1] };
                let hi = if u[k] == lv.len() { 1.0 } else { lv[u[k]] };
                p *= hi - lo;
            }
            let mut undercut = vec![false; b.links.len()];
            for (ui, &si) in unders.iter().zip(&s) {
                let l = &b.links[*ui];
                let st = l.strength.unwrap();
                p *= if si == 1 { st } else { 1.0 - st };
                if si == 1 && x(ix[l.from.as_str()]) {
                    let t = b.links.iter().position(|t| t.id == l.to).unwrap();
                    undercut[t] = true;
                }
            }
            if p == 0.0 {
                continue;
            }
            let consistent = ev.iter().all(|(li, _, _)| {
                let l = &b.links[*li];
                let src = ix.get(l.from.as_str()).is_none_or(|&f| x(f));
                let forced = fires[*li] && !undercut[*li] && src;
                !forced || x(ix[l.to.as_str()]) == (l.kind == LinkKind::Support)
            });
            if consistent {
                sum += p;
            }
        }
        out.insert(m, w * sum);
    }
    out
}

const REL: [f64; 3] = [0.6, 0.8, 0.95];
const PRI: [f64; 3] = [0.2, 0.5, 0.7];
const KINDS: [&str; 6] = ["excludes", "implies", "equiv", "exactly_one", "at_most_one", "at_least_one"];

#[allow(clippy::type_complexity)]
fn bundle() -> impl Strategy<Value = Value> {
    (1usize..=3, 0usize..=2, 0usize..=3, 0usize..=2, 0usize..=2).prop_flat_map(|(n, ne, nl, nu, nk)| {
        (
            Just((n, ne)),
            proptest::collection::vec((0usize..3, prop::option::weighted(0.3, 0usize..3), prop::bool::weighted(0.15)), n),
            proptest::collection::vec((0..n, prop::option::of(0usize..2)), ne),
            proptest::collection::vec((any::<bool>(), 0..(n + ne), 0..n, 0usize..3, any::<bool>()), nl),
            proptest::collection::vec((0..n, any::<prop::sample::Index>(), any::<bool>()), nu),
            proptest::collection::vec((0usize..6, proptest::collection::btree_set(0..n, 1..=n), prop::option::of(0.5f64..2.0)), nk),
            (0usize..3, 0usize..3),
        )
    })
    .prop_map(|((n, ne), cs, exs, ls, us, ks, (rho0, rho1))| {
        let rhos = [0.0, 0.4, 1.0];
        let claims: Vec<Value> = cs.iter().enumerate().map(|(i, (p, h, noticed))| {
            let mut c = json!({"id": format!("c{i}"), "text": "t", "prior": PRI[*p], "noticed": noticed});
            if let Some(h) = h { c["p_hint"] = json!(PRI[*h]); }
            c
        }).collect();
        let exhibits: Vec<Value> = exs.iter().enumerate().map(|(i, (c, g))| {
            let mut e = json!({"id": format!("e{i}"), "reading": {"claim": format!("c{c}")}});
            if let Some(g) = g { e["clusters"] = json!([format!("g{g}")]); }
            e
        }).collect();
        let mut links: Vec<Value> = ls.iter().enumerate().map(|(i, (sup, from, to, r, llr))| {
            let from = if *from < n { format!("c{from}") } else { format!("e{}", from - n) };
            let weight = if *llr { json!({"llr": -(1.0 - REL[*r]).ln()}) } else { json!({"reliability": REL[*r]}) };
            json!({"id": format!("l{i}"), "kind": if *sup { "support" } else { "contradict" }, "from": from, "to": format!("c{to}"), "weight": weight})
        }).collect();
        let nev = links.len();
        if nev > 0 {
            for (j, (d, target, strong)) in us.iter().enumerate() {
                links.push(json!({"id": format!("u{j}"), "kind": "undercut", "from": format!("c{d}"), "to": format!("l{}", target.index(nev)), "strength": if *strong { 1.0 } else { 0.5 }}));
            }
        }
        let constraints: Vec<Value> = ks.iter().enumerate().map(|(i, (k, cl, soft))| json!({
            "id": format!("k{i}"), "kind": KINDS[*k], "claims": cl.iter().map(|c| format!("c{c}")).collect::<Vec<_>>(),
            "mode": soft.map_or(json!("hard"), |w| json!({"soft": w}))
        })).collect();
        let _ = ne;
        json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"}, "claims": claims, "exhibits": exhibits,
               "links": links, "constraints": constraints,
               "clusters": [{"key": "g0", "rho": rhos[rho0]}, {"key": "g1", "rho": rhos[rho1]}]})
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(150))]

    #[test]
    fn composer_matches_generative_oracle(v in bundle()) {
        let b = Bundle::from_json(v).unwrap();
        let worlds = oracle(&b, 0.5);
        let z: f64 = worlds.values().sum();
        let n = b.claims.len();
        let all: Vec<String> = b.claims.iter().map(|c| c.id.clone()).collect();
        let mut q = Query::default();
        q.sets.insert("all".into(), all);
        q.focus = Some("c0".into());
        let out = WmcExact::default().compose(&b, &json!({}), &q, &Budget::default());
        if z <= 0.0 {
            prop_assert_eq!(out.unwrap_err().code, "E_ZERO_PARTITION");
            return Ok(());
        }
        let out = out.unwrap().outputs;
        prop_assert!(close(out["partition"].as_f64().unwrap(), z), "Z {} vs {}", out["partition"], z);
        for i in 0..n {
            let want: f64 = worlds.iter().filter(|(m, _)| (*m >> i) & 1 == 1).map(|(_, w)| w).sum::<f64>() / z;
            let got = out["marginal"][format!("c{i}")].as_f64().unwrap();
            prop_assert!(close(got, want), "marginal c{i}: {got} vs {want}");
        }
        let full = (1u32 << n) - 1;
        let pj = worlds.get(&full).copied().unwrap_or(0.0) / z;
        prop_assert!(close(out["joint"]["all"].as_f64().unwrap(), pj));
        let rest: f64 = worlds.iter().filter(|(m, _)| *m | 1 == full).map(|(_, w)| w).sum::<f64>() / z;
        if rest > 0.0 {
            prop_assert!(close(out["conditional"]["all"].as_f64().unwrap(), pj / rest));
        }
    }
}

// ---------------------------------------------------------------- closed forms

fn one(links: Value, clusters: Value, exhibits: Value) -> serde_json::Map<String, Value> {
    let b = Bundle::from_json(json!({"schema": "condorcet/bundle/v1", "adapter": {"id": "t", "version": "1"},
        "claims": [{"id": "c", "text": "t", "prior": 0.3}], "exhibits": exhibits, "links": links, "clusters": clusters})).unwrap();
    WmcExact::default().compose(&b, &json!({}), &Query::default(), &Budget::default()).unwrap().outputs
}

fn sup(id: &str, from: &str, r: f64) -> Value {
    json!({"id": id, "kind": "support", "from": from, "to": "c", "weight": {"reliability": r}})
}

#[test]
fn closed_forms_for_one_link_and_cluster_extremes() {
    let (pi, r) = (0.3f64, 0.9f64);
    let post = |k: i32| pi / (pi + (1.0 - pi) * (1.0 - r).powi(k));
    let ex = json!([{"id": "e1", "reading": {"claim": "c"}, "clusters": ["g"]}, {"id": "e2", "reading": {"claim": "c"}, "clusters": ["g"]}]);
    let links = json!([sup("l1", "e1", r), sup("l2", "e2", r)]);
    let p = |o: &serde_json::Map<String, Value>| o["marginal"]["c"].as_f64().unwrap();
    assert!(close(p(&one(json!([sup("l1", "e1", r)]), json!([]), ex.clone())), post(1)));
    assert!(close(p(&one(links.clone(), json!([{"key": "g", "rho": 0.0}]), ex.clone())), post(2)));
    // rho = 1: two copies of one source count once.
    assert!(close(p(&one(links, json!([{"key": "g", "rho": 1.0}]), ex)), post(1)));
    // llr ln 19 is reliability 18/19: odds x19.
    let o = one(json!([{"id": "l", "kind": "support", "from": "e1", "to": "c", "weight": {"llr": 19f64.ln()}}]), json!([]),
        json!([{"id": "e1", "reading": {"claim": "c"}}]));
    let odds = p(&o) / (1.0 - p(&o));
    assert!(close(odds, 19.0 * 0.3 / 0.7));
}

#[test]
fn video_clock_defeater_cuts_the_reading() {
    let v: Value = serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap();
    let mut b = Bundle::from_json(v).unwrap();
    let run = |b: &Bundle| WmcExact::default().compose(b, &json!({}), &Query::default(), &Budget::default()).unwrap().outputs;
    let with = run(&b)["marginal"]["org:clx/fct-a1"].as_f64().unwrap();
    b.links.retain(|l| l.kind != LinkKind::Undercut);
    let without = run(&b)["marginal"]["org:clx/fct-a1"].as_f64().unwrap();
    assert!(with < without, "{with} !< {without}");
}
