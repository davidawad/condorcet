//! `condorcet` command line. Stub: only `status` exists so far.

use condorcet_core::Envelope;
use serde_json::json;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1);
    let out = match cmd.as_deref() {
        Some("status") | None => Envelope::ok(json!({
            "name": "condorcet",
            "version": env!("CARGO_PKG_VERSION"),
            "composers": [],
        })),
        Some(other) => Envelope::err(
            "E_UNKNOWN_COMMAND",
            &format!("unknown command: {other}"),
            "condorcet status",
        ),
    };
    println!("{}", serde_json::to_string(&out).unwrap_or_default());
    if out.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
