//! The condorcet store: an append-only run log (`runs.ndjson`), one file
//! per run (`runs/<hex>.json`), a memo index (`memo/<hex>`, memo key to
//! run id) and a bundle cache (`bundles/<hex>.json`). Nothing is ever
//! rewritten: an identical run is a lookup, not a second line.
//!
//! Location: `$CONDORCET_HOME`, else `$XDG_DATA_HOME/condorcet`, else
//! `~/.local/share/condorcet`.

use condorcet_core::{content_id, Bundle, IdKind, RunRecord};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

/// A store failure; `code` is stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StoreError {}

fn io(path: &Path, e: &std::io::Error) -> StoreError {
    StoreError { code: "E_STORE_IO", message: format!("{}: {e}", path.display()) }
}

fn hex(id: &str) -> Result<&str, StoreError> {
    id.rsplit_once(':')
        .map(|(_, h)| h)
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| StoreError { code: "E_ID_INVALID", message: format!("not a content id: {id}") })
}

/// The default store root.
#[must_use]
pub fn default_root() -> PathBuf {
    if let Some(h) = std::env::var_os("CONDORCET_HOME") {
        return PathBuf::from(h);
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(x).join("condorcet");
    }
    std::env::var_os("HOME").map_or_else(|| PathBuf::from(".condorcet"), |h| PathBuf::from(h).join(".local/share/condorcet"))
}

impl Store {
    #[must_use]
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn ensure(&self, sub: &str) -> Result<PathBuf, StoreError> {
        let d = self.root.join(sub);
        fs::create_dir_all(&d).map_err(|e| io(&d, &e))?;
        Ok(d)
    }

    /// Cache a bundle under its content id.
    ///
    /// # Errors
    /// The store is not writable.
    pub fn put_bundle(&self, b: &Bundle) -> Result<String, StoreError> {
        let v = serde_json::to_value(b).map_err(|e| StoreError { code: "E_STORE_IO", message: e.to_string() })?;
        let id = content_id(IdKind::Bundle, &v);
        let p = self.ensure("bundles")?.join(format!("{}.json", hex(&id)?));
        if !p.exists() {
            fs::write(&p, v.to_string()).map_err(|e| io(&p, &e))?;
        }
        Ok(id)
    }

    /// # Errors
    /// `E_NOT_FOUND`, or the cached file is unreadable.
    pub fn get_bundle(&self, id: &str) -> Result<Bundle, StoreError> {
        let p = self.root.join("bundles").join(format!("{}.json", hex(id)?));
        let text = fs::read_to_string(&p).map_err(|_| StoreError { code: "E_NOT_FOUND", message: format!("no bundle {id}") })?;
        let v = serde_json::from_str(&text).map_err(|e| StoreError { code: "E_STORE_CORRUPT", message: e.to_string() })?;
        Bundle::from_json(v).map_err(|e| StoreError { code: "E_STORE_CORRUPT", message: e.to_string() })
    }

    /// Append a sealed run. Returns `false` when the run was already
    /// stored (same id), in which case nothing is written.
    ///
    /// # Errors
    /// The store is not writable, or the record is not sealed.
    pub fn append_run(&self, run: &RunRecord) -> Result<bool, StoreError> {
        let h = hex(&run.id)?.to_owned();
        let p = self.ensure("runs")?.join(format!("{h}.json"));
        if p.exists() {
            return Ok(false);
        }
        let line = serde_json::to_string(run).map_err(|e| StoreError { code: "E_STORE_IO", message: e.to_string() })?;
        fs::write(&p, &line).map_err(|e| io(&p, &e))?;
        let log = self.root.join("runs.ndjson");
        let mut f = OpenOptions::new().create(true).append(true).open(&log).map_err(|e| io(&log, &e))?;
        writeln!(f, "{line}").map_err(|e| io(&log, &e))?;
        let m = self.ensure("memo")?.join(hex(&run.memo_key)?);
        fs::write(&m, &run.id).map_err(|e| io(&m, &e))?;
        Ok(true)
    }

    /// # Errors
    /// `E_NOT_FOUND`, or the file is unreadable.
    pub fn get_run(&self, id: &str) -> Result<RunRecord, StoreError> {
        let p = self.root.join("runs").join(format!("{}.json", hex(id)?));
        let text = fs::read_to_string(&p).map_err(|_| StoreError { code: "E_NOT_FOUND", message: format!("no run {id}") })?;
        serde_json::from_str(&text).map_err(|e| StoreError { code: "E_STORE_CORRUPT", message: e.to_string() })
    }

    /// The run stored under a memo key, if any.
    #[must_use]
    pub fn find_memo(&self, memo_key: &str) -> Option<RunRecord> {
        let id = fs::read_to_string(self.root.join("memo").join(hex(memo_key).ok()?)).ok()?;
        self.get_run(id.trim()).ok()
    }

    /// Run ids in append order.
    #[must_use]
    pub fn run_ids(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("runs.ndjson"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["id"].as_str().map(str::to_owned))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("condorcet-store-{}-{n}", std::process::id()))
    }

    #[test]
    fn runs_are_append_only_and_memoised() {
        let s = Store::open(tmp());
        let run: RunRecord = serde_json::from_str(include_str!("../../../schemas/examples/run.video-clock.json")).unwrap();
        assert!(s.append_run(&run).unwrap());
        assert!(!s.append_run(&run).unwrap());
        assert_eq!(s.run_ids(), vec![run.id.clone()]);
        assert_eq!(s.get_run(&run.id).unwrap(), run);
        assert_eq!(s.find_memo(&run.memo_key).unwrap().id, run.id);
        assert_eq!(s.get_run("run:s256:00").unwrap_err().code, "E_ID_INVALID");
    }

    #[test]
    fn bundles_cache_by_content_id() {
        let s = Store::open(tmp());
        let v = serde_json::from_str(include_str!("../../../schemas/examples/bundle.video-clock.json")).unwrap();
        let b = Bundle::from_json(v).unwrap();
        let id = s.put_bundle(&b).unwrap();
        assert_eq!(s.put_bundle(&b).unwrap(), id);
        assert_eq!(s.get_bundle(&id).unwrap(), b);
    }
}
