//! What this build can run: the built-in composers and profiles.

use condorcet_composer::{Composer, Profile};
use serde_json::Value;

pub const COMPOSER_SPECS: [(&str, &str); 4] = [
    ("sat-core", condorcet_sat::SPEC),
    ("wmc-exact", condorcet_wmc::SPEC),
    ("dung", condorcet_dung::SPEC),
    ("condorcet-jury", condorcet_jury::SPEC),
];

pub const PROFILES: [(&str, &str); 2] = [
    ("gate-default", include_str!("../../../profiles/gate-default.json")),
    ("gate-strict", include_str!("../../../profiles/gate-strict.json")),
];

pub const SCHEMAS: [(&str, &str); 4] = [
    ("bundle", include_str!("../../../schemas/bundle.schema.json")),
    ("composer", include_str!("../../../schemas/composer.schema.json")),
    ("profile", include_str!("../../../schemas/profile.schema.json")),
    ("run", include_str!("../../../schemas/run.schema.json")),
];

/// A composer by `name@version` (or bare `name`, meaning the built-in version).
#[must_use]
pub fn composer(id: &str) -> Option<Box<dyn Composer>> {
    let (name, version) = id.split_once('@').unwrap_or((id, "1"));
    if version != "1" {
        return None;
    }
    match name {
        "sat-core" => Some(Box::new(condorcet_sat::SatCore::default())),
        "wmc-exact" => Some(Box::new(condorcet_wmc::WmcExact::default())),
        "dung" => Some(Box::new(condorcet_dung::Dung::default())),
        "condorcet-jury" => Some(Box::new(condorcet_jury::Jury::default())),
        _ => None,
    }
}

#[must_use]
pub fn composer_ids() -> Vec<String> {
    COMPOSER_SPECS.iter().map(|(n, _)| format!("{n}@1")).collect()
}

#[must_use]
pub fn spec(name: &str) -> Option<Value> {
    let name = name.split('@').next().unwrap_or(name);
    COMPOSER_SPECS.iter().find(|(n, _)| *n == name).and_then(|(_, s)| serde_json::from_str(s).ok())
}

#[must_use]
pub fn builtin_profile(name: &str) -> Option<Profile> {
    let name = name.split('@').next().unwrap_or(name);
    PROFILES
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, s)| serde_json::from_str(s).ok())
        .and_then(|v| Profile::from_json(v).ok())
}

#[must_use]
pub fn schema(name: &str) -> Option<Value> {
    SCHEMAS.iter().find(|(n, _)| *n == name).and_then(|(_, s)| serde_json::from_str(s).ok())
}
