//! The workspace has no dependency outside its own directory, so it can be
//! split out unchanged: every `path = ...` dependency in every manifest must
//! resolve inside the workspace root.

use std::path::{Component, Path, PathBuf};

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn manifests(dir: &Path, acc: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("readable dir").flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != ".git" && name != ".worktrees" {
                manifests(&p, acc);
            }
        } else if name == "Cargo.toml" {
            acc.push(p);
        }
    }
}

#[test]
fn no_path_dependency_leaves_the_workspace() {
    let root = normalize(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let mut ms = Vec::new();
    manifests(&root, &mut ms);
    assert!(!ms.is_empty());
    for m in ms {
        let text = std::fs::read_to_string(&m).expect("readable manifest");
        let dir = m.parent().expect("manifest has a parent");
        for part in text.split("path = \"").skip(1) {
            let rel = part.split('"').next().unwrap_or_default();
            let target = normalize(&dir.join(rel));
            assert!(
                target.starts_with(&root),
                "{} depends on {} outside {}",
                m.display(),
                target.display(),
                root.display()
            );
        }
    }
}
