//! The fact adapter boundary: the [`FactAdapter`] trait (for in-process
//! adapters and tests) and the executable protocol `condorcet/adapter/v1`,
//! which is the authoritative contract.
//!
//! # Protocol
//!
//! An adapter `<id>` is an executable `condorcet-adapter-<id>` found on
//! `PATH` (or named by `CONDORCET_ADAPTER_<ID>`, upper-cased, `-` as `_`).
//! Each command prints one envelope on stdout, `{"ok":true,"data":...}` or
//! `{"ok":false,"error":{"code","message","next"}}`; the exit status is not
//! read. The working directory is inherited.
//!
//! - `capabilities`: data `{"id","version","protocol":"condorcet/adapter/v1","load","write_back","profile"}`.
//! - `load --scope <json object>`: data is a Bundle v1. The scope is the
//!   adapter's to interpret; the engine passes `staged` (a claim's local id)
//!   and `proposed_path` for the gate.
//! - `profile <name>`: data is a profile v1 for a name the engine does not
//!   know (the adapter maps its own evidence profiles to engine gates).
//! - `write-back [--commit --expected-revision <rev>]`: stdin is
//!   `{"run": <run v1>, "scope": <scope>, "write_estimates": bool}`; data is
//!   a receipt `{"mode":"preview"|"commit","revision",...}`. Preview writes
//!   nothing.

use condorcet_core::{Bundle, RunRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub const PROTOCOL: &str = "condorcet/adapter/v1";

/// A typed adapter failure; `code` is stable and `next` is a command to try.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub next: String,
}

impl AdapterError {
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>, next: &str) -> Self {
        Self { code: code.into(), message: message.into(), next: next.into() }
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AdapterError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub load: bool,
    #[serde(default)]
    pub write_back: bool,
    #[serde(default)]
    pub profile: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteMode {
    Preview,
    Commit { expected_revision: String },
}

/// What the adapter wrote, or would write.
pub type WriteReceipt = Value;

/// Pure data in, pure data out. Adapters hold no engine state; the engine
/// holds no domain rules.
pub trait FactAdapter {
    fn id(&self) -> &str;
    /// # Errors
    /// The adapter could not answer.
    fn capabilities(&self) -> Result<Capabilities, AdapterError>;
    /// # Errors
    /// The scope is invalid or refused, or the source is unreadable.
    fn load(&self, scope: &Value) -> Result<Bundle, AdapterError>;
    /// A profile v1 for a name the engine does not know.
    ///
    /// # Errors
    /// `E_PROFILE_NOT_FOUND` when the adapter has none either.
    fn profile(&self, name: &str) -> Result<Value, AdapterError>;
    /// Append-only. Preview returns what would be written and the revision
    /// it was computed against.
    ///
    /// # Errors
    /// `E_REVISION_MISMATCH` when committing against a moved revision.
    fn write_back(
        &self,
        run: &RunRecord,
        scope: &Value,
        write_estimates: bool,
        mode: &WriteMode,
    ) -> Result<WriteReceipt, AdapterError>;
}

/// Read an adapter envelope. A bare object without `ok` is taken as data,
/// so a script that prints a Bundle is a legal `load`.
///
/// # Errors
/// The envelope's own error, or `E_ADAPTER_PROTOCOL` when the output is
/// not JSON.
pub fn read_envelope(stdout: &[u8]) -> Result<Value, AdapterError> {
    let v: Value = serde_json::from_slice(stdout).map_err(|e| {
        AdapterError::new("E_ADAPTER_PROTOCOL", format!("adapter output is not JSON: {e}"), "condorcet adapters")
    })?;
    match v.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(v.get("data").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let e = v.get("error").cloned().unwrap_or(Value::Null);
            Err(serde_json::from_value(e).unwrap_or_else(|_| {
                AdapterError::new("E_ADAPTER_ERROR", "adapter failed without a typed error", "condorcet adapters")
            }))
        }
        None => Ok(v),
    }
}

/// Check that a `load` result is a usable Bundle (the conformance test an
/// adapter author runs against their own output).
///
/// # Errors
/// `E_BUNDLE_PARSE` / `E_BUNDLE_INVALID`.
pub fn conform_bundle(data: Value) -> Result<Bundle, AdapterError> {
    Bundle::from_json(data).map_err(|e| AdapterError::new(e.code, e.message, "condorcet bundle --validate <file>"))
}

/// An adapter executable speaking the protocol.
#[derive(Debug, Clone)]
pub struct ExecAdapter {
    pub id: String,
    pub bin: PathBuf,
    pub timeout: Duration,
}

fn env_name(id: &str) -> String {
    format!("CONDORCET_ADAPTER_{}", id.to_uppercase().replace('-', "_"))
}

/// Every `condorcet-adapter-*` executable on `PATH`, as `(id, path)`.
#[must_use]
pub fn discover() -> Vec<(String, PathBuf)> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut out: Vec<(String, PathBuf)> = std::env::split_paths(&path)
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("condorcet-adapter-").map(|id| (id.to_owned(), e.path()))
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

impl ExecAdapter {
    /// Find adapter `id`: `$CONDORCET_ADAPTER_<ID>`, else
    /// `condorcet-adapter-<id>` on `PATH`.
    ///
    /// # Errors
    /// `E_ADAPTER_NOT_FOUND`.
    pub fn find(id: &str, timeout: Duration) -> Result<Self, AdapterError> {
        let bin = std::env::var_os(env_name(id))
            .map(PathBuf::from)
            .or_else(|| on_path(&format!("condorcet-adapter-{id}")))
            .ok_or_else(|| {
                AdapterError::new(
                    "E_ADAPTER_NOT_FOUND",
                    format!("no condorcet-adapter-{id} on PATH and {} unset", env_name(id)),
                    "condorcet adapters",
                )
            })?;
        Ok(Self { id: id.to_owned(), bin, timeout })
    }

    fn call(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Value, AdapterError> {
        let out = run_bounded(&self.bin, args, stdin, self.timeout)
            .map_err(|why| AdapterError::new("E_ADAPTER_UNAVAILABLE", why, "condorcet adapters"))?;
        read_envelope(&out)
    }
}

/// Run `bin args`, feeding `stdin`, for at most `timeout`; stdout on success.
fn run_bounded(bin: &Path, args: &[&str], stdin: Option<&[u8]>, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input).map_err(|e| e.to_string())?;
    }
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(out) => out.map(|o| o.stdout).map_err(|e| e.to_string()),
        Err(_) => {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            Err(format!("adapter timed out after {}s", timeout.as_secs()))
        }
    }
}

impl FactAdapter for ExecAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn capabilities(&self) -> Result<Capabilities, AdapterError> {
        let v = self.call(&["capabilities"], None)?;
        serde_json::from_value(v)
            .map_err(|e| AdapterError::new("E_ADAPTER_PROTOCOL", e.to_string(), "condorcet adapters"))
    }

    fn load(&self, scope: &Value) -> Result<Bundle, AdapterError> {
        let s = scope.to_string();
        conform_bundle(self.call(&["load", "--scope", &s], None)?)
    }

    fn profile(&self, name: &str) -> Result<Value, AdapterError> {
        self.call(&["profile", name], None)
    }

    fn write_back(
        &self,
        run: &RunRecord,
        scope: &Value,
        write_estimates: bool,
        mode: &WriteMode,
    ) -> Result<WriteReceipt, AdapterError> {
        let input = json!({"run": run, "scope": scope, "write_estimates": write_estimates}).to_string();
        match mode {
            WriteMode::Preview => self.call(&["write-back"], Some(input.as_bytes())),
            WriteMode::Commit { expected_revision } => self.call(
                &["write-back", "--commit", "--expected-revision", expected_revision],
                Some(input.as_bytes()),
            ),
        }
    }
}

/// The built-in `file` adapter: scope `{"path": "<bundle.json>"}`. Read-only.
#[derive(Debug, Clone, Default)]
pub struct FileAdapter;

impl FactAdapter for FileAdapter {
    fn id(&self) -> &'static str {
        "file"
    }

    fn capabilities(&self) -> Result<Capabilities, AdapterError> {
        Ok(Capabilities {
            id: "file".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL.into(),
            load: true,
            write_back: false,
            profile: false,
        })
    }

    fn load(&self, scope: &Value) -> Result<Bundle, AdapterError> {
        let path = scope.get("path").and_then(Value::as_str).ok_or_else(|| {
            AdapterError::new("E_SCOPE_INVALID", "file adapter needs scope.path", "condorcet compose --bundle <file>")
        })?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| AdapterError::new("E_BUNDLE_UNREADABLE", format!("{path}: {e}"), "ls <file>"))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AdapterError::new("E_BUNDLE_PARSE", format!("{path}: {e}"), "condorcet bundle --validate <file>"))?;
        conform_bundle(v)
    }

    fn profile(&self, name: &str) -> Result<Value, AdapterError> {
        Err(AdapterError::new("E_PROFILE_NOT_FOUND", format!("no profile {name}"), "condorcet profile list"))
    }

    fn write_back(&self, _: &RunRecord, _: &Value, _: bool, _: &WriteMode) -> Result<WriteReceipt, AdapterError> {
        Err(AdapterError::new(
            "E_WRITE_BACK_UNSUPPORTED",
            "the file adapter is read-only; runs are kept in the condorcet store",
            "condorcet explain <run id>",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const EXAMPLE: &str = include_str!("../../../schemas/examples/bundle.video-clock.json");

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn exec(bin: PathBuf, secs: u64) -> ExecAdapter {
        ExecAdapter { id: "t".into(), bin, timeout: Duration::from_secs(secs) }
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("condorcet-adapter-test-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn enveloped_and_bare_bundles_load() {
        let d = tmp();
        let f = d.join("b.json");
        std::fs::write(&f, EXAMPLE).unwrap();
        let bare = script(&d, "bare", &format!("cat {}", f.display()));
        let env = script(&d, "env", &format!("printf '{{\"ok\":true,\"data\":'; cat {}; printf '}}'", f.display()));
        let a = exec(bare, 5).load(&json!({})).unwrap();
        let b = exec(env, 5).load(&json!({})).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.claims.len(), 3);
    }

    #[test]
    fn typed_errors_pass_through_and_garbage_is_protocol_error() {
        let d = tmp();
        let err = script(&d, "err", r#"echo '{"ok":false,"error":{"code":"E_DOMAIN_WALL","message":"m","next":"n"}}'"#);
        assert_eq!(exec(err, 5).load(&json!({})).unwrap_err().code, "E_DOMAIN_WALL");
        let junk = script(&d, "junk", "echo nope");
        assert_eq!(exec(junk, 5).load(&json!({})).unwrap_err().code, "E_ADAPTER_PROTOCOL");
        let slow = script(&d, "slow", "sleep 20");
        assert_eq!(exec(slow, 1).load(&json!({})).unwrap_err().code, "E_ADAPTER_UNAVAILABLE");
    }

    #[test]
    fn scope_and_write_back_reach_the_executable() {
        let d = tmp();
        let echo = script(
            &d,
            "echo",
            r#"if [ "$1" = write-back ]; then printf '{"ok":true,"data":{"args":"%s","stdin":' "$*"; cat; printf '}}'; else printf '{"ok":true,"data":{"args":"%s"}}' "$1 $2"; fi"#,
        );
        let a = exec(echo, 5);
        assert_eq!(a.profile("p").unwrap()["args"], "profile p");
        let run: RunRecord = serde_json::from_str(include_str!("../../../schemas/examples/run.video-clock.json")).unwrap();
        let r = a
            .write_back(&run, &json!({"staged": "x"}), false, &WriteMode::Commit { expected_revision: "git:1".into() })
            .unwrap();
        assert_eq!(r["args"], "write-back --commit --expected-revision git:1");
        assert_eq!(r["stdin"]["scope"]["staged"], "x");
        assert_eq!(r["stdin"]["run"]["id"], json!(run.id));
    }

    #[test]
    fn non_conforming_bundle_is_rejected() {
        let mut v: Value = serde_json::from_str(EXAMPLE).unwrap();
        v["links"][0]["weight"] = json!({"reliability": 0.4});
        assert_eq!(conform_bundle(v).unwrap_err().code, "E_BUNDLE_INVALID");
    }

    #[test]
    fn file_adapter_loads_and_refuses_write_back() {
        let d = tmp();
        let f = d.join("b.json");
        std::fs::write(&f, EXAMPLE).unwrap();
        let b = FileAdapter.load(&json!({"path": f})).unwrap();
        assert_eq!(b.adapter.id, "org");
        assert_eq!(FileAdapter.load(&json!({})).unwrap_err().code, "E_SCOPE_INVALID");
    }
}
