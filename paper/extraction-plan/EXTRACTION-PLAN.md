# EXTRACTION-PLAN — moving `shrinkray-delta-codec` out cleanly

Step-by-step playbook the user can execute **later, not today**. The trigger
condition is at the bottom of this file (§ When to actually run this).
Until then, this is reference material; nothing here should be run yet.

Assumes the four sibling docs in `paper/extraction-plan/` have been read:
- `AUDIT.md` — the dep + coupling inventory
- `NAMING.md` — recommended name: `delta-mip`
- `LICENSE-DECISION.md` — recommended license: Apache-2.0
- `PUBLIC-README.md` — the README that will ship in the new repo

Conventions below: `$NEW` = path where the new repo will be created, e.g.
`/home/thankamax/projects/delta-mip`. `$SR` = shrinkray repo,
`/home/thankamax/projects/shrinkray`. Substitute on use.

---

## Phase 0 — preflight

Do **all** of this in shrinkray's `main` (or whatever release branch is
clean), **before** any extraction commands.

```bash
cd "$SR"
git status                       # must be clean — no untracked, no staged
git fetch --all                  # refresh remotes
git log -1 --pretty=oneline      # record the commit you're cutting from
git tag pre-delta-mip-extraction # save a return-to point in case of regret
```

Verify the crate builds + tests pass *here* in the shrinkray workspace.
If it doesn't, fix that first — extracting a broken crate just exports
the bug.

```bash
cargo test -p shrinkray-delta-codec
cargo clippy -p shrinkray-delta-codec -- -D warnings
cargo doc   -p shrinkray-delta-codec --no-deps
```

## Phase 1 — extract the crate history with `git filter-repo`

`git filter-repo` is the modern replacement for `git filter-branch`;
install via `pipx install git-filter-repo` (or distro package). The
key behaviour: it rewrites commits to contain only the subset of paths
we keep, including renames. It also clears the `origin` remote by design
(safety net so you don't accidentally push the rewrite back to shrinkray).

```bash
# Work on a fresh clone so the shrinkray repo on disk isn't touched.
cd /home/thankamax/projects
git clone "$SR" delta-mip-extract
cd delta-mip-extract

# Sanity: confirm we have the crate's history with renames included.
git log --follow --oneline -- crates/shrinkray-delta-codec | head

# Rewrite the repo to contain ONLY the crate, with the crate's own path
# as the new repo root. --path-rename lifts `crates/shrinkray-delta-codec/`
# up to the root of the new repo.
git filter-repo \
  --path crates/shrinkray-delta-codec/ \
  --path-rename crates/shrinkray-delta-codec/: \
  --force

# Result: ./Cargo.toml is the crate's own Cargo.toml; ./src/ is the source;
# every commit's diffs are now relative to those paths.
git log --oneline | head
ls
```

Spot-check the rewrite:

```bash
# Should show only crate-relevant files in every commit.
git log --stat | head -50

# Should NOT contain references to shrinkray-core / shrinkray-cli / src-tauri.
git log --all --diff-filter=A --name-only --pretty=format: | sort -u | head
```

If anything looks wrong, delete `delta-mip-extract/` and re-run. The
shrinkray repo is untouched.

## Phase 2 — repackage as a standalone crate

Apply the per-deliverable cleanups from `AUDIT.md`:

```bash
cd /home/thankamax/projects/delta-mip-extract

# 2a — rewrite Cargo.toml. The workspace inheritance has to go; pin direct.
# Use the AUDIT.md §6 checklist as your guide. Sample target Cargo.toml:
```

```toml
[package]
name        = "delta-mip"
version     = "0.1.0"
edition     = "2021"
license     = "Apache-2.0"
description = "Residual-coded image codec with one-bitstream lossy/lossless selection."
repository  = "https://github.com/<owner>/delta-mip"
authors     = ["Thanuka Sehasna Perera"]
keywords    = ["compression", "codec", "image", "residual", "bcn"]
categories  = ["compression", "multimedia::images"]

[dependencies]
serde     = { version = "1",   features = ["derive"] }
anyhow    = "1"
sha2      = "0.10"
image_dds = "0.7"
zstd      = "0.13"

[dev-dependencies]
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }

[[example]]
name = "delta-mip-bench"
path = "examples/delta_codec_bench.rs"
```

```bash
# 2b — apply doc-string cleanups (AUDIT.md §4.4). Optional renames
# (BcResidualFormat → BcFormat, CodecSpace::Bc3Byte → BcByte) belong in
# 0.1 if you're going to do them at all — bitstream identity is decided
# here.

# 2c — add Apache-2.0 LICENSE + NOTICE.
curl -o LICENSE https://www.apache.org/licenses/LICENSE-2.0.txt
cat > NOTICE <<'EOF'
delta-mip
Copyright 2026 Thanuka Sehasna Perera
EOF

# 2d — drop in the README from paper/extraction-plan/PUBLIC-README.md.
cp "$SR/paper/extraction-plan/PUBLIC-README.md" README.md

# 2e — verify.
cargo build
cargo test
cargo clippy -- -D warnings
cargo doc --no-deps

# 2f — commit. Use a "release: split off delta-mip from shrinkray" commit,
# not a squash — keeping the rewritten history is the whole point of using
# filter-repo.
git add -A
git commit -m "chore: split delta-mip out of shrinkray monorepo (Apache-2.0, v0.1.0)"
```

## Phase 3 — push to GitHub

```bash
# Create the empty GitHub repo first via gh CLI (no `--source`, no `--push`
# yet — we want our own freshly-rewritten history to land first).
gh repo create delta-mip --public --description "Residual-coded image codec; one bitstream covers both byte-exact restore and lossy distribution."

# Now wire the local repo to the new remote and push.
git remote add origin https://github.com/<owner>/delta-mip.git
git branch -M main
git push -u origin main
git push --tags  # if you tagged 0.1.0 locally
```

## Phase 4 — CI on the new repo

Create `.github/workflows/ci.yml` in the new repo:

```yaml
name: ci
on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  test:
    name: ${{ matrix.os }} — cargo test
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-features

  lint:
    name: clippy + fmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --all-targets --all-features -- -D warnings

  docs:
    name: cargo doc
    runs-on: ubuntu-latest
    env:
      RUSTDOCFLAGS: "-D warnings"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --no-deps --all-features
```

Three jobs, all required for PR merge. The cross-OS matrix on `test` is
non-optional — the BC encoder (`image_dds`) has historically had platform-
specific quirks; we need to catch them before users do. The `docs` job
fails on broken intradoc links, which is the single most common
post-extraction regression source.

Optional v0.1 nice-to-have (skip for first release):

- `cargo-deny` (`deny.toml` with the standard "no GPL deps, no yanked
  crates" preset).
- `cargo-msrv` workflow to pin a minimum-supported Rust version.
- `cargo-bench` workflow against a checked-in `criterion` suite. The
  current bench is an `example`, not a `criterion` harness — converting
  it is a separate PR.

## Phase 5 — point shrinkray at the new crate

The shrinkray side. Two stages: a *path dep* for the dev loop, a *git
dep with a pinned tag* (or eventually a crates.io version) for release.

### 5a — initial cutover (path dep, for the active week post-split)

```bash
cd "$SR"
# Move the freshly-split repo to where shrinkray's Cargo.toml expects it.
mv /home/thankamax/projects/delta-mip-extract /home/thankamax/projects/delta-mip

# Remove the crate from shrinkray's workspace, point at the new path.
# Edit Cargo.toml:
#   - remove "crates/shrinkray-delta-codec" from [workspace.members]
#   - change [workspace.dependencies].shrinkray-delta-codec to:
#       shrinkray-delta-codec = { path = "../delta-mip" }
#     (or rename the dep itself to `delta-mip` and find/replace consumers)

# Delete the in-tree copy.
git rm -r crates/shrinkray-delta-codec
cargo build      # entire workspace must still build
cargo test -p shrinkray-core    # consumers must still pass
```

### 5b — release cutover (git dep with rev pin)

When `delta-mip` cuts a release tag (say `v0.1.0`):

```toml
# shrinkray's Cargo.toml — switch from path to git+rev pin so the release
# build is reproducible and doesn't depend on a sibling working tree.
delta-mip = { git = "https://github.com/<owner>/delta-mip", tag = "v0.1.0" }
```

### 5c — long-term (crates.io dep)

Once `delta-mip` has been published to crates.io and held up under at
least one minor bump:

```toml
delta-mip = "0.1"
```

## When to actually run this

**Not yet. Specifically:**

1. **Wait for measurements.** The parallel measurement agent has to land
   real numbers (corpus, ratios, byte-exact rates) before this crate is
   worth publishing. A research-preview crate with no measurements is
   noise; the same crate with a results page is a citable artifact.

2. **Wait for paper draft 1.** The README's "Status" section links to a
   results page. That page needs to exist (even in rough draft form)
   before the README isn't lying. The paper draft also disciplines the
   API: writing the methodology section often catches "wait, that
   parameter shouldn't have been exposed" type mistakes that are much
   cheaper to fix pre-publish than post.

3. **Wait for the rename + docstring pass.** §6 of `AUDIT.md` is the
   pre-publish checklist. Don't burn the v0.1.0 tag on a half-cleaned
   crate; the version-bump cost of cleaning up later is high (semver-
   breaking renames within the first weeks of release feels amateur).

4. **Do all the v0.1 source edits *inside the shrinkray monorepo first*.**
   That way the `git filter-repo` extraction carries the cleanups with
   it, and the crate's first commit on the new repo is the
   "split off" commit, not "split off → immediately rewrite half the
   files". Cleaner history; nicer to read for first-time contributors;
   `git blame` keeps working across the split.

**Concrete trigger:** when the measurements PR + the paper draft 1 PR
have both landed in shrinkray, AND the §6 checklist in `AUDIT.md` is
green, run Phase 0 onward. Estimated wall-clock once you start: half a
day for Phases 0-4, another half-day for the shrinkray-side cutover
and CI shakeout.

## Rollback

If something goes wrong post-push:

```bash
# Local-only rewrites are cheap to discard.
rm -rf /home/thankamax/projects/delta-mip-extract

# If the GitHub repo is already public but full of mistakes, delete it
# via the gh CLI (irreversible — make sure):
#   gh repo delete <owner>/delta-mip --yes
# Then re-run from Phase 0.

# If shrinkray's workspace cut was done already and you want to undo:
cd "$SR"
git reset --hard pre-delta-mip-extraction
# (The tag we set in Phase 0. It exists for exactly this reason.)
```
