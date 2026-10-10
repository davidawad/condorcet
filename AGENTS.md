# assaymark

Rust workspace for a fact-composition engine. See README.md for layout.

- Local-only: no remote, no publication, until David says otherwise.
- Gate: `cargo build && cargo test && cargo clippy --all-targets -- -D warnings`.
- Schemas under `schemas/` are the contract; examples must validate (tests enforce it).
- The engine commits to no domain: no source classes, `lr_plus` or business-record rules here. Those belong to adapters.
- Tasks live in `.beads/` (br). The design doc is `docs/architecture/fact-composition.md` in the dotfiles repo.
- Work in a linked worktree under `.worktrees/`.
