# Picabia seams in upstream VectorCraft files

Every edit Picabia makes to a file that comes from upstream VectorCraft is a *seam*: as few lines as
possible, calling into Picabia code, marked `// PICABIA: <reason>` (`rg "PICABIA:"` lists them).
When an upstream merge conflicts, resolve it seam by seam with this table, then run `cargo xtask ci`.

| File | Function / item | Why | How to re-apply |
|---|---|---|---|
| `.github/workflows/release.yml` (`release` job), `sonarcloud.yml`, `freebsd.yml`, `windows-arm64.yml` | top job | Need storytold signing secrets, SonarCloud token or its runners/schedules | Re-add `if: github.repository == 'storytold/vectorcraft'` under the job name (4 lines in all). `ci.yml`, `nix.yml`, `packaging-lint.yml`, `affinity-corpus.yml` and `print.yml` run unchanged on GitHub-hosted runners. |
| `Cargo.toml` | `[workspace.dependencies]` | Picabia crates are workspace dependencies | One `vectorcraft-<crate> = { path = "crates/<crate>" }` line per Picabia crate under a `# PICABIA:` comment (`vectorcraft-modifiers`, `vectorcraft-stencil`). Workspace members come from the `crates/*` glob. |
| `xtask/src/layers.rs` | `TABLE` | `cargo xtask layers` must know each Picabia crate's layer | One row per Picabia crate under a `// PICABIA:` comment: `("modifiers", Class::Layer(2))`, `("stencil", Class::Layer(3))`. |
| `crates/engine/src/cmd.rs` | `ordinary_isolation` | clippy 1.96 flags upstream's boolean (`nonminimal_bool`); `cargo xtask ci` fails on pristine upstream | `#[allow(clippy::nonminimal_bool)]` on the function. Drop it once upstream rewrites the expression. |
