//! `condorcet`: compose evidence with declared, versioned techniques.
//!
//! Every command prints one envelope, `{"ok":true,"data":...}` or
//! `{"ok":false,"error":{"code","message","next"}}`. `gate` always exits 0
//! (its caller reads the envelope); other commands exit 2 on error.

mod args;
mod cmd;
mod registry;

use condorcet_core::Envelope;
use std::process::ExitCode;

/// A typed command failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: String,
    pub message: String,
    pub next: String,
}

impl Fail {
    pub fn new(code: &str, message: impl Into<String>, next: &str) -> Self {
        Self { code: code.into(), message: message.into(), next: next.into() }
    }
}

impl From<args::Usage> for Fail {
    fn from((message, next): args::Usage) -> Self {
        Self { code: "E_USAGE".into(), message, next }
    }
}

impl From<condorcet_adapter::AdapterError> for Fail {
    fn from(e: condorcet_adapter::AdapterError) -> Self {
        Self { code: e.code, message: e.message, next: e.next }
    }
}

impl From<condorcet_store::StoreError> for Fail {
    fn from(e: condorcet_store::StoreError) -> Self {
        Self::new(e.code, e.message, "condorcet status")
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let is_gate = raw.first().map(String::as_str) == Some("gate");
    let out = match args::Args::parse(raw) {
        Ok(a) => match cmd::dispatch(&a) {
            Ok(data) => Envelope::ok(data),
            Err(f) => Envelope::err(&f.code, &f.message, &f.next),
        },
        Err((m, n)) => Envelope::err("E_USAGE", &m, &n),
    };
    println!("{}", serde_json::to_string(&out).unwrap_or_default());
    if out.is_ok() || is_gate {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
