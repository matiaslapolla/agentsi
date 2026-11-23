# AgentSI north star plan

Spec: `docs/specs/agentsi-north-star.md`. Docs only, one branch `north-star`. The owner runs U5 on the owner's machine after the merge to `main`.

## U1 — The north star
- **Files:** `docs/product.md`
- **Steps:**
  1. Write the nine headings and the Stage line of the spec in their order.
  2. Write objective 1 and objective 2 with their roadmap sections, signals, queries and review date.
  3. Write the later sections and the owner override under `## Later`.
  4. Write the monthly check, the conditions to stop and the outside list under `## Review`.
- **Check:** each section of `ROADMAP.md` appears under one objective, `## Later` or `## Out of scope`.

## U2 — The roadmap
- **Files:** `ROADMAP.md`
- **Steps:**
  1. Rename "macOS and the switch-over" to "The switch-over" and say that it is in progress.
  2. Remove the macOS item, the remote client item and the mobile app item.
- **Check:** `grep -niE "macos|mobile app|remote client" ROADMAP.md` prints nothing.

## U3 — The brief
- **Files:** `docs/plans/agentsi-north-star-brief.json`
- **Steps:**
  1. Write the brief with the objectives `o1` and `o2`, the roster roles and `north_stars`.
  2. Run `corepack enable && pnpm install --frozen-lockfile`.
  3. Run a throwaway script with `node --import tsx` that calls `validateBrief` and prints the failures.
  4. Delete the script. Commit only the JSON.
- **Check:** the script prints no failures.

## U4 — The terms
- **Files:** `CONTEXT.md`
- **Steps:**
  1. Add **North star** after **Brief**, in the style of the file.
- **Check:** `grep -n "^\*\*North star\*\*" CONTEXT.md` prints one line.

## U5 — The registration, on the owner's machine
- **Files:** none in the repo. The core writes the data root, by default `~/.local/share/agentsi/data`.
- **Steps:**
  1. On `main` in the root of the AgentSI repo, stage the brief and the roster:

     ```sh
     stage=$(mktemp -d)
     cp docs/plans/agentsi-north-star-brief.json "$stage/brief.json"
     echo '["pm","dev","qa"]' > "$stage/roster.json"
     ```

  2. Run the dry run. It writes nothing and prints the roster change and the diff:

     ```sh
     agentsi roles apply "$stage" --register --product agentsi --name "AgentSI" --path "$PWD"
     ```

  3. Read the diff. Stop if a line is not expected.
  4. Only after the diff looks correct, apply it:

     ```sh
     agentsi roles apply "$stage" --register --product agentsi --name "AgentSI" --path "$PWD" --yes
     ```

  5. Validate the result:

     ```sh
     agentsi roles validate --product agentsi
     ```

- **Check:** `agentsi roles validate` passes, and `agentsi roles --product agentsi` lists `pm`, `dev` and `qa`.
