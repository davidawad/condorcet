# assaymark

A configurable engine for how true a body of information is. It takes a
**bundle** (claims, the exhibits that bear on them, the support / contradict /
undercut links between them, declared constraints and independence clusters),
a **composer** declared as JSON (a logical, argumentation or probabilistic
technique) and a **profile** (an ordered cascade of composer steps plus
gates), and emits an append-only, content-addressed **composition run**: a
score or verdict for each claim and for sets of claims, plus the evidence for
it (unsat cores, drop sets, extensions, circuits, seeds).

Techniques disagree on purpose. The technique is a declared, versioned,
swappable thing, not code buried in a scorer. The adapter turns a domain's
rules of evidence into numbers; the engine composes numbers and never learns
what a business record is.

Status: **v0.1 scaffold.** Local-only: no remote, not published. The
schemas, content-addressed ids and the `{ok, data | error}` envelope exist;
no composer is implemented yet.

## Layers

| Layer | Noun | ID form | Declared as |
|---|---|---|---|
| L0 | Bundle (claims, exhibits, links, constraints, clusters) | `bundle:s256:<hex>` | produced by a fact adapter, `schemas/bundle.schema.json` |
| L1 | Composer | `composer:<name>@<n>` | `composers/<name>/composer.json`, `schemas/composer.schema.json` |
| L2 | Profile | `profile:<name>@<n>` | `profiles/<name>.json`, `schemas/profile.schema.json` |
| L3 | Composition run | `run:s256:<hex>` | append-only record, `schemas/run.schema.json` |

Every id names its kind. Output is `{"ok": true, "data": ...}` or
`{"ok": false, "error": {"code", "message", "next"}}`.

## Layout

```
crates/core       ids, envelope (Bundle and run types arrive with the beads that use them)
crates/adapter    FactAdapter trait and adapter protocol (stub)
crates/composer   Composer trait, spec loader, cascade runner (stub)
crates/store      append-only run log, memo, bundle cache (stub)
crates/cli        the `assaymark` binary (stub: `assaymark status`)
schemas/          bundle, composer, profile, run JSON schemas, plus examples/
composers/        one directory per composer (empty so far)
profiles/         one file per profile (empty so far)
```

The workspace has no dependency outside its own directory; a test enforces it.

## Build

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

## Design

The design lives in the author's dotfiles repository
(`docs/architecture/fact-composition.md`); planned work is tracked in this
repository's `.beads/` store.

## License

Dual-licensed under MIT or Apache-2.0, at your option. See `LICENSE-MIT` and
`LICENSE-APACHE`.
