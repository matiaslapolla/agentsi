---
status: done
date: 2026-10-21
---

# Guide from a demo product

Unit 4 of `docs/plans/agentsi-open-source.md`.

## Problem

`make guide` takes its captures from the owner's data root. The guide HTML and its images show the names of the owner's real products, the owner's name and the owner's paths. The web fixture `apps/web/fixtures/demo` also tells a meeting of a real product. The public repo cannot ship either one.

## Decision

- `make guide` builds a throwaway demo environment, captures from it, and removes it at the end.
- The demo environment has these parts:
  - a temporary data root with a registry of three fictional products: MiApp (`miapp`, with a stories block), Tienda (`tienda`) and Blog (`blog`),
  - one small git repo for each product, with a README, one doc and a few commits by an author named "Demo",
  - a separate database, `agentsi_guide`, in the local Postgres, made with the migrations and dropped at the end,
  - one meeting of MiApp, written with `writeMinutes` from the fixture, and one story written with `saveStory`.
- The script runs every CLI command with `AGENTSI_HOME` and `AGENTSI_DATABASE_URL` set to the demo environment.
- For the web screenshots, the script starts its own web app on port 3364 with the same environment, and a separate Next.js `distDir`. It stops that web app at the end. The owner's web app on 3361 is not touched.
- The prompts in the captures show the demo paths as `~/dev/<product>` and the code checkout as `~/agentsi`.
- The fixture in `apps/web/fixtures/demo` tells a MiApp meeting. `pnpm demo:seed` and the guide use the same fixture.

## Out of scope

- The other open-source units (owner config, hardening, voice licences, public tree).
- A real meeting with model calls during `make guide`.

## Acceptance criteria

- [x] `make guide` runs with no real product in the registry of the demo environment, and it leaves no temporary data root, database or web process after it ends.
- [x] `docs/guide/README.md` (it was `index.html`) and every file in `docs/guide/img/` contain no name of the owner's products, no name of the owner and no path under a home directory, in any case.
- [x] `apps/web/fixtures/demo` contains none of the words in the previous criterion.
- [x] The guide keeps its nine terminal frames and its four web screenshots, and each one shows the demo data.
- [x] `make guide` does not change the owner's data root, the owner's database or the owner's web app on 3361.
- [x] Typecheck, lint and the tests pass.
