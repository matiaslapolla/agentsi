# The Rust rebuild is licensed under Apache-2.0, and the repo changes from MIT at the switch-over

**Status:** accepted 2026-11-02

## Context

The repo is MIT today, and `packages/voice` is GPL-3.0-or-later. The rebuild copies patterns from Herdr (Apache-2.0) and Hex (MIT), and it uses `gpui-component` (Apache-2.0). The project wants contributors and a public release.

## Decision

- Each crate in `crates/` declares `license = "Apache-2.0"` from its first commit.
- The TypeScript code stays MIT until the switch-over.
- In the switch-over commit, `LICENSE` becomes the Apache-2.0 text, and `NOTICE` names Herdr, Hex and each bundled font.
- No crate links GPL code. `espeak-ng` runs as a separate program.
- `cargo deny check licenses` runs in CI.

## Consequences

- Apache-2.0 gives contributors a patent grant, and it matches the license of the two main references.
- Until the switch-over, the repo has two licenses. `crates/README.md` says so.
- The GPL voice package leaves with the switch-over.
- Speech out depends on an external `espeak-ng` until a permissive phonemizer exists.
- Rejected: MIT for the crates, because the references and `gpui-component` are Apache-2.0 and the owner chose it. GPL-3.0, because it blocks reuse by other tools.
