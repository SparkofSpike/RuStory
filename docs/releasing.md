# Releasing

Nothing here has been done yet — the project has not published a crate. This page is the
checklist for the day it does, written now because most of it is facts that are annoying to
rediscover under pressure.

## What blocks a publish today

**The names are settled, which they were not.** `rustory` on crates.io is a 2018 disk-usage tool
(MIT, not yanked), so the workspace publishes as `rustory-vn-*` while the repository stays
`RuStory` and the binaries stay `rustory` / `rustory-editor`. The reasoning is recorded in
[ADR 0016](adr/0016-three-names.md); the practical consequence for a publish is that every
package name carries the `-vn` suffix and every documented command does not.

**Licence texts are in every crate directory.** `LICENSE-MIT` and `LICENSE-APACHE` are copied
into each of the eight library crates and both apps, because Cargo looks for them beside each
`Cargo.toml` and a crate archive that names a licence it does not contain is a defect. The copies
are byte-identical to the workspace-root files; a check below fails if they drift.

**The applications are not meant to be published.** `rustory` and `rustory-editor` are binaries
for this repository's users; the crates worth publishing are the libraries. Both already carry
`publish = false`, so a workspace-wide `cargo publish` skips them.

## The order that works

```console
# 1. The gate everything else depends on.
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace

# 2. The licence copies have not drifted from the root.
#    (A crate archive naming a licence it does not contain is a packaging defect, and a
#    copy that silently fell behind is the same defect with better intentions.)
for d in crates/* apps/*; do
  cmp LICENSE-MIT "$d/LICENSE-MIT" && cmp LICENSE-APACHE "$d/LICENSE-APACHE"
done

# 3. What would actually go in the archive. Read it; a missing file here is a file users
#    will not get.
cargo package -p rustory-vn-core --list

# 4. A full build of the packaged crate, which catches a path that works in the workspace
#    and not outside it.
cargo package -p rustory-vn-core
```

Publish in dependency order — `rustory-vn-core`, then `rustory-vn-script` / `rustory-vn-audio` /
`rustory-vn-render`, then `rustory-vn-edit`, then the gameplay crates. Cargo refuses a publish
whose dependencies are not on the registry yet, and the error names the missing one.

`rustory-vn-gameplay-wasm` depends on `rustory-vn-core` and on `wasmi`; it publishes after the core
and nothing else in the workspace depends on it.

## Versioning

The workspace shares one version (`workspace.package.version`). That is fine while the
crates move together and wrong the moment one of them stabilises ahead of the others: a
`0.2` of `rustory-vn-core` that changes `GameplaySystem` should not force a `0.2` of
`rustory-vn-render` that changed nothing.

Until a third party depends on a specific crate, the shared version is a feature: one number
to bump, one changelog. Revisit when that stops being true, and split the versions then —
not speculatively.

## What a release needs that the repository does not have

- A `docs.rs` check. Documentation that fails to build on docs.rs fails for a reason that
  does not appear locally, usually a missing `--all-features`.
- A named maintainer. `Copyright (c) 2026 RuStory contributors` is honest and also means no
  single person answers a security report.

`CHANGELOG.md` exists now, with an `[Unreleased]` section. It stops being honest the moment a
change lands without a line there, which is the reason it was started before the first release
rather than after.
