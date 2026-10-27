# Releasing

Nothing here has been done yet — the project has not published a crate. This page is the
checklist for the day it does, written now because most of it is facts that are annoying to
rediscover under pressure.

## What blocks a publish today

**The crate name is taken.** `rustory` on crates.io is a 2018 disk-usage tool (MIT, not
yanked). Local package and binary names are unaffected, but a publish needs a free name.
Measured with `cargo search` at the time of writing: `rustory-vn`, `rustory-engine` and
`rustory-core` were free.

**The licence texts are not in the crate directories.** `license = "MIT OR Apache-2.0"` is a
correct SPDX declaration and it is what crates.io displays, but the convention — and the
expectation of anyone vendoring the source — is that a crate archive contains the texts it
claims to be licensed under. Cargo looks for `LICENSE-MIT` / `LICENSE-APACHE` beside each
`Cargo.toml`, and the copies live at the workspace root. Either symlink them into each crate
directory before packaging or accept an archive that names a licence it does not include.

**The applications are not meant to be published.** `rustory` and `rustory-editor` are
binaries for this repository's users; the crates worth publishing are the libraries. Add
`publish = false` to the two apps rather than relying on memory.

## The order that works

```console
# 1. The gate everything else depends on.
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace

# 2. What would actually go in the archive. Read it; a missing file here is a file users
#    will not get.
cargo package -p rustory-core --list

# 3. A full build of the packaged crate, which catches a path that works in the workspace
#    and not outside it.
cargo package -p rustory-core
```

Publish in dependency order — `rustory-core`, then `rustory-script` / `rustory-audio` /
`rustory-render`, then `rustory-edit`, then the gameplay crates. Cargo refuses a publish
whose dependencies are not on the registry yet, and the error names the missing one.

## Versioning

The workspace shares one version (`workspace.package.version`). That is fine while the
crates move together and wrong the moment one of them stabilises ahead of the others: a
`0.2` of `rustory-core` that changes `GameplaySystem` should not force a `0.2` of
`rustory-render` that changed nothing.

Until a third party depends on a specific crate, the shared version is a feature: one number
to bump, one changelog. Revisit when that stops being true, and split the versions then —
not speculatively.

## What a release needs that the repository does not have

- A changelog. `git log` is not one; a reader wants to know what changed for them.
- A `docs.rs` check. Documentation that fails to build on docs.rs fails for a reason that
  does not appear locally, usually a missing `--all-features`.
- A named maintainer. `Copyright (c) 2026 RuStory contributors` is honest and also means no
  single person answers a security report.
