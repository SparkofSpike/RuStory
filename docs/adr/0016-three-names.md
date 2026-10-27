# ADR 0016 — Three names, each with one job

- **Status**: Accepted, implemented
- **Date**: 2026-10-27
- **Open question**: what is this thing called?

---

## Context

By the time the engine had a working vertical slice, three naming questions had collided:

1. **The repository** is `SparkofSpike/RuStory`. That is the project's identity and every link
   into it; renaming it breaks clones, links and anything already written down.
2. **The crate name** `rustory` is taken on crates.io — a 2018 disk-usage tool — so publishing
   under it is impossible.
3. **The command** a person types is `rustory`. A bin name is an interface: changing it breaks
   every script, README line and muscle memory that already uses it.

An earlier session tried to satisfy all three with one string and could not. The resolution is
that they were never one string.

## Decision

| Thing | Name | Why |
|---|---|---|
| Repository, project identity | **RuStory** | Already public; the name people link to |
| Crates on crates.io | `rustory-vn-*` | Unique, and the suffix says what it is |
| Binaries | `rustory`, `rustory-editor` | What people type; unchanged from the first release |
| Cargo package of the player | `rustory-vn` with `[[bin]] name = "rustory"` | The two are allowed to differ, and here they must |

The workspace has one crate per layer, all under the same prefix: `rustory-vn-core`,
`rustory-vn-script`, `rustory-vn-render`, `rustory-vn-audio`, `rustory-vn-edit`,
`rustory-vn-gameplay-{timer,quiz,wasm}`, plus `apps/rustory-vn` and `apps/rustory-vn-editor` (package) with `rustory-editor` as its binary.

Gameplay crates written by third parties are **not** required to use the prefix. The contract is
the trait, not the name; a creator's crate is theirs.

## Why not rename everything to one word

- Renaming the repository would break links for a cosmetic gain. The project is public and the
  owner said explicitly not to.
- Renaming the binaries would break the interface with no benefit to anyone: `cargo install
  rustory-vn` producing `rustory` is a normal thing for a Cargo project, and the note in the
  README says so.
- Keeping the crate name `rustory` would mean not publishing. A published name is a promise the
  project can keep; a taken one is not.

## Consequences

- Once published, `cargo install rustory-vn` will install a command called `rustory`. The README
does not say so yet, because the crates are not published — see `docs/releasing.md` for what is
still blocking that. A documented command that cannot run is the one thing this project's
writing rules forbid.
- Paths in documentation use crate names (`crates/rustory-vn-core/…`) and commands use binary
  names (`rustory check …`). Both spellings appear in the same paragraph, deliberately.
- A future rename of the repository stays possible; nothing in the code depends on it, only
  `repository`/`homepage` in `Cargo.toml`.

## Verification

- CI builds the workspace and runs `target/debug/rustory …` in the README smoke job, which is
  what proves the bin name survived the package rename.
- Every command in the README uses `rustory` or `rustory-editor`; every file path uses
  `rustory-vn-*`. A reader who notices the difference finds this ADR.
