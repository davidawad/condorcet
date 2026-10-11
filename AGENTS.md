# condorcet

Rust workspace for a fact-composition engine. See README.md for layout.

- Public repository; licensed MIT OR Apache-2.0.
- Gate: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`.
- Schemas under `schemas/` are the contract; examples must validate (tests enforce it).
- The engine commits to no domain: no source classes, `lr_plus` or business-record rules here. Those belong to adapters.
- Task tracking is kept outside this repository; `.beads/issues.jsonl` is not committed.
- Work in a linked worktree under `.worktrees/`.
