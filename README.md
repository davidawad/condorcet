# condorcet

A fact-composition engine. A fact adapter supplies a **bundle**: claims, the exhibits that bear on them, weighted support, contradict and undercut links, constraints and independence clusters. A **profile** then runs a cascade of declared **composers** over that bundle. The result is an append-only, content-addressed **run** record. The engine contains no domain rules: adapters turn their own rules of evidence into numbers, and the engine composes the numbers.

## Composers (built in)

| id | family | answers |
|---|---|---|
| `sat-core@1` | logical | Can the asserted claims and hard relations hold together? If not, what are the minimal unsatisfiable cores (deletion MUS, MARCO enumeration; rustsat + batsat)? |
| `wmc-exact@1` | probabilistic | Exact marginal, joint and conditional probabilities, expected false members, and the partition. Uses encoding S and its own component-caching DPLL weighted model counter. |
| `dung@1` | argumentation | Grounded, preferred and stable labellings, with attacker chains. |
| `condorcet-jury@1` | probabilistic | The probability that the majority of readings is correct. Correlated clusters are collapsed, and the ceiling is reported. |

Each composer is declared in `composers/<name>/composer.json` (schema `schemas/composer.schema.json`). Each one is property-tested against brute-force enumeration.

## CLI

```text
condorcet status
condorcet composer list | show <name> | validate <file>
condorcet profile list | show <name> | validate <file>
condorcet adapters
condorcet bundle  (--bundle F | --adapter A --scope J) [--validate]
condorcet compose (--bundle F | --adapter A --scope J) --profile P [--focus C] [--set name=a,b] [--commit --expected-revision R] [--write-estimates]
condorcet joint   --bundle F <claim>...
condorcet core    --bundle F [--max-cores N]
condorcet argue   --bundle F [--semantics grounded|preferred|stable]
condorcet jury    --bundle F
condorcet gate    --adapter A --profile P --staged ID [--proposed PATH]
condorcet explain run:s256:<hex>
condorcet resolve <run:|bundle:|composer:|profile: id>
```

Every command prints `{"ok":true,"data":...}` or `{"ok":false,"error":{"code","message","next"}}`. `gate` always exits 0. Its data is `{"verdict":"pass"|"hold"|"refuse","reasons":[...],"run":"run:s256:..",...}`.

Profiles are looked up in this order: a file path, then `$CONDORCET_PROFILES` directories, then the built-in `profiles/`, and finally the adapter's `profile <name>`. The store lives at `$CONDORCET_HOME` (default `~/.local/share/condorcet`). It holds `runs.ndjson`, `runs/`, `memo/` and `bundles/`.

## Adapters

An adapter is an executable `condorcet-adapter-<id>` on `PATH` that speaks `condorcet/adapter/v1`. It supports `capabilities`, `load --scope <json>`, `profile <name>` and `write-back [--commit --expected-revision R]`. The protocol is documented in `crates/adapter/src/lib.rs`. The built-in `file` adapter reads a bundle from disk.

## Development

Work in `.worktrees/`. The gate is `cargo test` plus `cargo clippy --all-targets -- -D warnings`. The schemas under `schemas/` are the contract.
