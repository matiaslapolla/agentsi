# AgentSI desktop: general roles for all products

**Status:** approved 2026-11-21 · **Source:** owner request of 2026-11-21 ("crear roles generales y que estén accesibles a los distintos proyectos")

## Problem

The five global roles (pm, dev, qa, scout, social) come with the build. A new role can only be a custom role of one product (`workspace/<p>/agents/<agent>.md`). The app has no way to make a role once and add it to several products.

## Goal

- The roles screen can create, edit and delete a general role.
- A general role shows in the roster picker of each product.
- The owner can copy a custom role of a product to a general role.

## Non-goals

- Edits of the five builtin roles from the app. A product overlay (`extends:`) stays the way to change them.
- Sync of general roles between hosts.

## Terms

- **General role:** a role file in `<data root>/plugin/agents/` that is not one of the five builtin roles. `roles/files.rs` already reads these files as global roles (`GLOBAL_ORDER`).

## Design

### Core

- `agentsi-agency/src/roles` gets `general.rs` with `create`, `update`, `delete` and `promote`.
  - `create {id, name, description, body}` writes `<data root>/plugin/agents/<id>.md` with the front matter of the builtin roles. The id has the rules of `roster.rs`. An id of a builtin role returns `invalid`.
  - `delete {id}` refuses a role that a product roster lists, and names the products.
  - `promote {product, agent}` copies a custom role of a product to a general role, and the product roster then uses the general role.
- `agentsi-proto` gets `role.general.{list,create,update,delete,promote}`. Each write sends `data.changed {kind: "roles"}`.
- The MCP tool `role_memory` and meetings see general roles as they see global roles today. No change.

### App (`screens/roles.rs`)

- The screen gets a section "General roles" above the roster of the product.
- "New general role" opens a form: id, name, description, and the body in a multi-line input.
- The roster picker of a product lists builtin roles, then general roles, then the custom roles of the product.
- A custom role row has "Make general" in its menu.
- The palette gets "Roles: New general role".

## Acceptance criteria

- [ ] A general role `designer` made in the app is the file `<data root>/plugin/agents/designer.md`.
- [ ] Product A and product B can both add `designer` to their rosters, and a meeting of A cites it.
- [ ] `create` with the id `dev` returns `invalid`.
- [ ] `delete` of a role that product A lists is refused, and the error names A.
- [ ] "Make general" on a custom role of A moves the file, and A's roster still shows the role.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
