# Contributing to hpc

Thanks for taking the time to contribute. This project follows a **Gerrit-style,
patch-based review workflow**: review happens at the granularity of a single
*change* (one commit), each change carries a stable `Change-Id`, and history is
kept linear by rebasing rather than merging. The sections below describe that
model and how to practise the same discipline whether you are pushing to a
Gerrit server or opening a pull request on the GitHub mirror.

## TL;DR

```bash
scripts/setup.sh              # one-time: toolchain + native deps
git checkout -b my-change origin/main
# ... make ONE logical change ...
scripts/ci-local.sh           # fmt + clippy + test + build — must be green
git commit                    # write a good message; keep the Change-Id trailer
git push origin HEAD:refs/for/main    # Gerrit
# or, on the GitHub mirror:
git push origin my-change && gh pr create
```

## The change-based model

In Gerrit the unit of review is a **change**, represented by exactly one commit.
You do not open a branch full of "fix review comments" commits; instead you
**amend** the single commit and push a new *patch set*. This keeps `main` linear
and every landed commit independently reviewable, bisectable and revertable.

- **One logical change per commit.** If you find yourself writing "and also" in
  the commit message, split it into two changes.
- **Amend, don't append.** Address review feedback with
  `git commit --amend` (or an interactive rebase for a stack) and re-push. Each
  push creates a new patch set on the same change.
- **Rebase, don't merge.** Never merge `main` into your change. Rebase onto the
  latest `origin/main` so history stays flat.

### Change-Id

Every commit must carry a `Change-Id` trailer — this is how Gerrit ties patch
sets of the same change together across amends and rebases. Install the hook
once and it is added automatically:

```bash
# For a Gerrit remote (adjust host/port to your server):
gitdir=$(git rev-parse --git-dir)
scp -p -P 29418 <user>@<gerrit-host>:hooks/commit-msg "$gitdir/hooks/"
chmod +x "$gitdir/hooks/commit-msg"
```

A commit message then looks like:

```
hpc-diag: flag ephemeral state directories

/var mounted on tmpfs silently loses persisted state across reboots,
which surfaced as "config resets itself" reports. Detect ephemeral
filesystems under /var and emit a warning-level anomaly so operators
see the cause in the diagnostic bundle.

Change-Id: I3a5f0c9b7e2d1148a6f0e2c4b9d8a7f6c5e4d3b2
```

## Commit message conventions

- **Subject line**: `<crate>: <imperative summary>`, ≤ 72 chars, no trailing
  period. The crate prefix (`hpc-core`, `hpc-ffi`, `hpc-diag`, `scripts`, `ci`, …)
  scopes the change.
- **Blank line**, then a body that explains *why*, wrapped at ~72 columns.
- **Trailers** last: `Change-Id:`, and where relevant `Bug:`, `Signed-off-by:`.

## Review scoring

Gerrit gates a change on two independent labels:

| Label          | Range        | Meaning                                             |
|----------------|--------------|-----------------------------------------------------|
| `Code-Review`  | `-2 … +2`    | A reviewer approves the change (`+2` to submit).     |
| `Verified`     | `-1 … +1`    | CI (or a human) confirms it builds and tests pass.   |

A change is submittable only with a `Code-Review +2` **and** `Verified +1`, and
no `-2`. `-2` is a hard veto; `+1`/`-1` on Code-Review are non-blocking
opinions. On the GitHub mirror these map to an approving PR review plus green
required status checks (see below).

## The verification gate

CI enforces exactly what [`scripts/ci-local.sh`](scripts/ci-local.sh) runs
locally, so run it before you push:

1. `cargo fmt --all --check` — formatting.
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints, zero warnings.
3. `cargo test --workspace` — unit + integration tests.
4. `cargo build --workspace --release` — release build.

The same steps run in [`.github/workflows/ci.yml`](.github/workflows/ci.yml)
and in the [`Jenkinsfile`](Jenkinsfile); benchmarks run separately in
[`.github/workflows/bench.yml`](.github/workflows/bench.yml) and post results to
the pull request.

## Engineering conventions

These are enforced by review and clippy — see the "Engineering conventions"
section of the [README](README.md):

- No `unwrap()`/`expect()` in library code; propagate `hpc_core::Result` with `?`.
- `unsafe` lives only in `hpc-ffi`, and every `unsafe` block carries a
  `// SAFETY:` comment justifying it.
- New public behaviour comes with tests; new modules come with doc comments.

## GitHub mirror mapping

If you contribute through the GitHub mirror instead of Gerrit:

- Keep the **one-logical-change-per-PR** rule; squash-merge so `main` gets a
  single clean commit.
- Treat an **approving review** as `Code-Review +2` and **required green
  checks** (`ci`) as `Verified +1`.
- Rebase your branch on `main` rather than merging `main` into it.
