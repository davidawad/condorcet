//! A small argument reader: positionals plus `--flag value` and boolean
//! `--flag` (for the names in `BOOLEAN`). Repeated flags accumulate.

use std::collections::BTreeMap;

const BOOLEAN: [&str; 4] = ["commit", "write-estimates", "validate", "help"];

#[derive(Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    pub flags: BTreeMap<String, Vec<String>>,
}

/// A usage error: `(message, next command)`.
pub type Usage = (String, String);

impl Args {
    /// # Errors
    /// A value flag without a value.
    pub fn parse(raw: impl IntoIterator<Item = String>) -> Result<Self, Usage> {
        let mut a = Self::default();
        let mut it = raw.into_iter();
        while let Some(x) = it.next() {
            if let Some(name) = x.strip_prefix("--") {
                let (name, inline) = match name.split_once('=') {
                    Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                    None => (name.to_owned(), None),
                };
                let value = if BOOLEAN.contains(&name.as_str()) {
                    inline.unwrap_or_else(|| "true".into())
                } else if let Some(v) = inline {
                    v
                } else {
                    it.next().ok_or_else(|| (format!("--{name} needs a value"), "condorcet help".to_owned()))?
                };
                a.flags.entry(name).or_default().push(value);
            } else {
                a.positional.push(x);
            }
        }
        Ok(a)
    }

    #[must_use]
    pub fn get(&self, k: &str) -> Option<&str> {
        self.flags.get(k).and_then(|v| v.last()).map(String::as_str)
    }

    #[must_use]
    pub fn all(&self, k: &str) -> Vec<&str> {
        self.flags.get(k).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default()
    }

    #[must_use]
    pub fn has(&self, k: &str) -> bool {
        self.get(k).is_some_and(|v| v != "false")
    }

    /// # Errors
    /// The flag is absent.
    pub fn need(&self, k: &str, next: &str) -> Result<&str, Usage> {
        self.get(k).ok_or_else(|| (format!("--{k} is required"), next.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_booleans_and_positionals() {
        let a = Args::parse(
            ["gate", "--adapter", "org", "--staged=f1", "--commit", "--set", "a=x", "--set", "b=y"].map(String::from),
        )
        .unwrap();
        assert_eq!(a.positional, vec!["gate"]);
        assert_eq!((a.get("adapter"), a.get("staged")), (Some("org"), Some("f1")));
        assert!(a.has("commit") && !a.has("validate"));
        assert_eq!(a.all("set"), vec!["a=x", "b=y"]);
        assert!(Args::parse(["--profile".to_owned()]).is_err());
    }
}
