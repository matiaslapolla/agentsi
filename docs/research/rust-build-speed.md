# Faster Rust builds for this workspace

**Status:** research · **Date:** 2026-11-21 · **Questions:** (1) Do "blaze", "cinder" and "ra-jit" exist, and do they apply to cargo? (2) Which established tools speed up the builds of this repo? (3) What is the gain, the cost, the memory impact and the aarch64 16K-page risk of each? (4) What should the dev machine, the cloud agents and CI do first?

Sources are primary: official docs, tool READMEs and source, and this repo. Access date for every link: 2026-11-21. Nothing was compiled. "Unverified" marks a claim without a primary source. Gain numbers come from each tool's own benchmark.

Repo facts: 907 packages in `Cargo.lock`, 60 integration test files, stable 1.99.1. The dev machine has GCC 16.1.1 and GNU ld 2.46, and no mold, lld, wild or sccache.

## 1. Summary

Ranked by gain for each unit of effort:

| # | Action | Dev (aarch64) | Cloud agents | CI |
|---|---|---|---|---|
| 1 | Use mold or wild in place of GNU ld | yes, largest gain | no (lld is the default) | no |
| 2 | Use sccache to share dependency builds between worktrees | yes | only with a remote bucket | optional |
| 3 | Build libghostty-vt once and link it with `AGENTSI_GHOSTTY_VT_LIB` | yes | yes | already cached |
| 4 | Use `cargo check -p` and `cargo clippy -p` in agent loops | yes | yes | n/a |
| 5 | Remove the separate `cargo check` step, set `save-if`, and use nextest | n/a | n/a | yes |
| 6 | Merge integration test files into one binary for each crate | yes | yes | yes |
| 7 | Generate a workspace-hack crate with cargo-hakari | later | later | later |

Do not adopt now:

- blaze: its daemon needs several times cargo's memory, and the dev box OOMs already.
- Cranelift and `-Zthreads`: both need nightly, and the repo pins stable.
- Cinder: it gives no speed on Linux.

Keep `jobs = 2` in `.worktrees/.cargo/config.toml`. Never share one `target-dir` between worktrees.

## 2. The three named tools

**blaze** exists. Repo: [indexable-inc/blaze](https://github.com/indexable-inc/blaze), from ix (ix.dev), which says Y Combinator F26 backs it. It is Apache-2.0 or MIT, has about 2.4k stars, and was created on 2026-10-19. Its daemon compiles from rust-analyzer's analysis and MIR with Cranelift or LLVM. It links with its own linker. It replaces `cargo build/check/run/test/bench` ([README](https://github.com/indexable-inc/blaze/blob/main/README.md)).

- Gain: edits rebuild 24x faster on Linux aarch64 and 11x faster on Linux x86-64, as the geometric mean over real projects. These are v1 numbers. "v2 has not been measured yet" ([benchmarks](https://github.com/indexable-inc/blaze/blob/main/docs/benchmarks.md)). The first build is not faster.
- Memory: peak memory for an edit is 3.9x cargo's on Linux aarch64. bevy takes 17.8 GB under blaze and 6.7 GB under cargo. bun takes 27.7 GB ([benchmarks](https://github.com/indexable-inc/blaze/blob/main/docs/benchmarks.md#how-much-memory-and-disk-it-takes)). One 16 GB test passed only with `CARGO_BUILD_JOBS=2` (9.6 GB). The default was killed at 14 GB ([platforms](https://github.com/indexable-inc/blaze/blob/main/docs/platforms.md)).
- Maturity: "It's young." It uses its own pinned nightly, not the project's pin. Its borrow checker "finds about two thirds of rustc's borrow errors" ([usage](https://github.com/indexable-inc/blaze/blob/main/docs/usage.md)).
- Fit: build scripts and proc macros are supported ([support](https://github.com/indexable-inc/blaze/blob/main/docs/support.md)). The aarch64 benchmarks ran on Graviton4. 16K-page support is unverified, and its issues have no hit for "page size" or "asahi". The installer downloads from `blaze-releases.ix.dev`.
- Verdict: real and promising, but too young and too memory-hungry for a 16 GB box with GPUI.

**ra-jit** is the old name of blaze. The README says: "blaze was called ra-jit until 2026-10-24." `github.com/indexable-inc/ra-jit` redirects to blaze. There is no separate tool.

**Cinder** exists. Repo: [CapSoftware/cinder](https://github.com/CapSoftware/cinder) (Cap screen recorder team). It is MIT, has about 100 stars, and the last commit is from 2026-08-16. It wraps cargo and reuses results that it proves are still valid ([README](https://github.com/CapSoftware/cinder/blob/main/README.md)).

- Linux: "Linux and Windows use the strict Cargo proxy, preserving command behavior without artifact acceleration." So it gives nothing on any machine of this repo.
- Maturity: "not yet distributed as a stable release". You build it from source.

No other Rust build tool named "blaze" exists. [rnza0u/blaze](https://github.com/rnza0u/blaze) is an unrelated monorepo task runner. "Blaze" is also Google's internal name for Bazel ([Bazel FAQ](https://bazel.build/about/faq)), which does not replace cargo.

## 3. Established options

### 3.1 Linker

- x86_64: Rust 1.90 made lld the default on `x86_64-unknown-linux-gnu`. ripgrep's incremental debug rebuild linked about 7x faster, and the end-to-end time dropped about 40%. A clean debug build was about 20% faster ([Rust blog](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable)). CI and the cloud agents already get this.
- aarch64: "Non-`x86_64-unknown-linux-gnu` Linux targets use the slow system linker" ([Cargo book](https://doc.rust-lang.org/cargo/guide/build-performance.html)). `-Clinker-features=+lld` is stable on no other target ([rustc book](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-features)). So the dev box links with GNU ld (bfd), and you must pick a faster linker yourself.
- mold 3.0 (2026-10-05) supports ARM64. It claims 4.9x faster than lld and 1.9x faster than wild at the median ([README](https://github.com/rui314/mold)). GCC 12.1 or later accepts `-fuse-ld=mold`.
- wild 0.11.0 (2026-10-24) supports AArch64 Linux and has no incremental linking yet ([README](https://github.com/wild-linker/wild)). GCC 16.1 or later accepts `-fuse-ld=wild` ([usage](https://github.com/wild-linker/wild/blob/main/USAGE.md)).
- 16K pages: on AArch64, mold defaults to a 65536-byte page ([source](https://github.com/rui314/mold/blob/main/elf/arch/arm64.rs)). wild does the same ([source](https://github.com/wild-linker/wild/blob/main/libwild/src/args/elf.rs)). Asahi requires LOAD alignment of at least `0x4000` ([Asahi](https://asahilinux.org/docs/sw/broken-software/)). Both meet it. Neither issue tracker has an open Asahi bug.
- Memory: unverified. With `jobs = 2`, at most two links run at once.
- Cold VM: no gain on x86_64, because lld is already in use.

### 3.2 sccache

- What it does: it wraps rustc and caches outputs on local disk (10 GB by default) or in S3, R2 or the GitHub Actions cache ([README](https://github.com/mozilla/sccache)). v0.19.0 ships aarch64 and x86_64 musl binaries.
- Limits: sccache does not cache `bin`, `dylib`, `cdylib` or `proc-macro` crates. It does not cache incremental crates by default, and those include the workspace members in dev ([Rust notes](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)). So it speeds up the 894 registry dependencies, not edits.
- Worktrees: `SCCACHE_BASEDIRS` strips the checkout path before it hashes. The key includes the source contents. So two worktrees share dependency outputs without the false reuse of a shared `target/`. A crate that bakes in `env!("CARGO_MANIFEST_DIR")` keeps the other checkout's path. `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS` fixes that.
- Gain: no primary measurement exists. A new worktree's dependency build should become mostly cache hits.
- Memory: a hit runs no rustc, so peak memory drops.
- Cold VM: it helps only with a remote bucket that the proxy allows.

### 3.3 Target dir and `build-dir`

- `build.build-dir` is stable since Cargo 1.91 ([release notes](https://blog.rust-lang.org/2025/10/30/Rust-1.91.0/)). It moves intermediate artifacts and supports `{workspace-path-hash}` ([config](https://doc.rust-lang.org/cargo/reference/config.html#buildbuild-dir)).
- That hash differs for each worktree. So it is safe, but it shares nothing and gives no speed.
- A shared `target-dir` gave a false artifact reuse (`docs/plans/desktop-modules-log.md`). Use sccache to share work.

### 3.4 Debug info and profiles

- `debug = "line-tables-only"` and `package."*" debug = false` are already set. They give faster codegen, faster links and a smaller `target/` ([Cargo book](https://doc.rust-lang.org/cargo/guide/build-performance.html)). Keep them.
- `split-debuginfo = "unpacked"` works on stable Linux ([rustc book](https://doc.rust-lang.org/rustc/codegen-options/index.html#split-debuginfo)). With line tables only, the gain is small (unverified).
- `[profile.dev.package."*"] opt-level = 1` makes tests run faster and cold dependency builds slower ([profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)). Cold VMs lose.
- Incremental compilation is on in dev. Cargo turns it off when `CI` is set ([config](https://doc.rust-lang.org/cargo/reference/config.html#buildincremental)). Cloud agents rebuild after edits, so keep it on there.

### 3.5 Nightly-only options

- Cranelift: it is a nightly rustup component, and it is supported on Linux AArch64 and x86_64. SIMD through `std::arch` is partial, and unwinding is experimental ([README](https://github.com/rust-lang/rustc_codegen_cranelift)). On Zed, it is 12% faster than LLVM with `line-tables-only` ([project goal](https://goals.rust-lang.org/2026/improve-cg_clif-performance.html)).
- `-Zthreads=8`: it is "up to 50%" faster, and it uses up to 35% more memory. Known bugs include deadlocks ([Rust blog](https://blog.rust-lang.org/2023/11/09/parallel-rustc/)). It is still nightly ([Cargo book](https://doc.rust-lang.org/cargo/guide/build-performance.html)).
- `feature-unification = "workspace"` and `[lints.cargo] unused_dependencies` also need nightly (`cargo -Z help` on 1.99.1).

### 3.6 Less code

- Dependencies: cargo-machete runs on stable. It is fast but imprecise and has false positives ([README](https://github.com/bnjbvr/cargo-machete)). cargo-udeps needs nightly to run ([README](https://github.com/est31/cargo-udeps)). Trim default features of heavy crates by hand.
- cargo-hakari: it builds one workspace-hack crate so each dependency compiles with one feature set. It claims builds up to 1.7x faster in total. It costs a checked-in `Cargo.lock` and a CI check for drift ([docs](https://docs.rs/cargo-hakari/latest/cargo_hakari/)). It helps when `-p` builds and `--workspace` builds alternate. The Makefile and the gate do that.
- Test binaries: "Each integration test results in a separate executable binary". The Cargo book advises one integration test split into modules ([targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#integration-tests)). 60 files mean 60 links. Merge each crate's `tests/*.rs` into `tests/it/main.rs`.

### 3.7 Commands and tests

- `cargo check` skips "the final step of code generation" ([cargo check](https://doc.rust-lang.org/cargo/commands/cargo-check.html)). With `-p <crate>`, it skips GPUI crates that the change does not touch.
- nextest runs test binaries in parallel. It was 1.4x to 3.4x faster on nine projects ([benchmarks](https://nexte.st/docs/benchmarks/)). It does not speed up the build, and it does not run doctests.

### 3.8 CI caching

- Swatinem/rust-cache v2 caches `~/.cargo` and the dependency part of `./target`, keyed on `Cargo.lock` and the manifests. It does not cache the workspace crates, and it sets `CARGO_INCREMENTAL=0` ([README](https://github.com/Swatinem/rust-cache)). `save-if` limits saves to `main`.
- sccache-action sets `SCCACHE_GHA_ENABLED=true` and `RUSTC_WRAPPER=sccache` ([README](https://github.com/Mozilla-Actions/sccache-action)). No primary benchmark compares it with rust-cache, so time both first.

## 4. Recommendation with config

### 4.1 Dev machine (Asahi, aarch64, 16 GB)

1. Install mold with `pacman -S mold`. The `extra` repo has mold 3.0.0 for aarch64.
2. Put the linker in the user config, `~/.cargo/config.toml`. Then CI and the cloud agents, which do not have mold, are not affected.

```toml
[target.aarch64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=mold"]   # or "-fuse-ld=wild" (GCC 16.1+)

[build]
rustc-wrapper = "sccache"
```

3. Set these in the shell profile:

```sh
export SCCACHE_BASEDIRS="$HOME/dev/personal/productos/agentsi:$HOME/dev/personal/productos/agentsi/.worktrees"
export SCCACHE_CACHE_SIZE="20G"
```

4. Keep `[build] jobs = 2` in `.worktrees/.cargo/config.toml`. Keep `CARGO_BUILD_JOBS=2` in the main checkout.
5. sccache does not cache what a build script runs, so each worktree runs the Zig build again. Copy `libghostty-vt.a` once from `target/debug/build/agentsi-vt-*/out/zig-out/lib/`. Then point the worktree config at it:

```toml
# .worktrees/.cargo/config.toml
[env]
AGENTSI_GHOSTTY_VT_LIB = "/home/matiaslapolla/.cache/agentsi/ghostty-vt/<commit>/libghostty-vt.a"
```

This path skips the pin check in `build.rs`. Put the `ghostty.lock` commit in the path, and update the path when the lock changes.

6. A change to `rustflags` or `rustc-wrapper` rebuilds everything once.
7. Before and after, time one cold worktree build and one edit-rebuild with `cargo build --timings` and `/usr/bin/time -v`.

### 4.2 Cloud agents (Ubuntu 24.04 x86_64, 4 cores, cold VM)

1. lld is already the default. Do not add a linker.
2. Run `cargo check -p <crate>` and `cargo clippy -p <crate>` in the loop. Run `cargo test -p <crate>` and the workspace gate once at the end.
3. Keep incremental on. The agent rebuilds after each edit.
4. Allow the hosts of a cold build: crates.io, the ort binaries, the Ghostty tarball and Zig.
5. If one cold build costs too much, put sccache on S3 or R2 ([S3 docs](https://github.com/mozilla/sccache/blob/main/docs/S3.md)). Allow the bucket in the proxy:

```sh
export RUSTC_WRAPPER=sccache SCCACHE_BUCKET=<bucket> SCCACHE_REGION=auto \
  SCCACHE_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com   # plus AWS_* credentials
```

6. A prebuilt `libghostty-vt.a` in the image or the bucket, named by `AGENTSI_GHOSTTY_VT_LIB`, removes the Zig build.

### 4.3 CI (`.github/workflows/rust.yml`)

1. Remove `cargo check --workspace --locked`. `cargo clippy --all-targets` type-checks the same code, so the check step repeats work (inferred, measure it).
2. Save the cache only on `main`. PR runs then do not evict it:

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    key: zig-${{ env.ZIG_VERSION }}-${{ hashFiles('crates/agentsi-vt/ghostty.lock') }}
    save-if: ${{ github.ref == 'refs/heads/main' }}
```

3. Run tests with `cargo nextest run --workspace --locked`, and add `cargo test --doc --workspace` for doctests. Install it with `taiki-e/install-action@nextest`.

## 5. Open gaps

- No number here was measured on this repo.
- blaze on a 16K-page kernel is untested.
- The memory of mold and wild on a GPUI test link is unmeasured.
- The download host of `ort` binaries was not checked.
