# AgentSI desktop: the model and the effort of the desk conversation

**Status:** approved 2026-11-22 · **Source:** owner request of 2026-11-22 ("En ask the front desk, tener los ids de los modelos de memoria es complicado. Podría haber un <select> para ello")

## Problem

The desk session runs `claude -p --model <m>` with the model and the effort of the `pm` role of the product (`crates/agentsi-core/src/desk/mod.rs` `plan`). The owner can change them only in a role file or in the Models form of the setup, and both take a model id as free text. The owner must know the ids.

## Goal

- The desk conversation has one select for the model and one select for the effort.
- The owner never types a model id for the desk.
- The choice of a product stays after a restart of the app and of the core.

## Non-goals

- A change of the models of the meeting roles or of the Models form of the setup.
- A list of models that the app reads from the network.
- A free text field for a model id.

## Terms

- **Desk model**: the model of the desk session of one product.
- **Desk effort**: the effort of the desk session of one product.
- **Role default**: the model or the effort of the `pm` role of the product. With no choice, the desk session uses it.

## Decisions

- D1. The model list holds the aliases of `claude --model`: Fable, Opus, Sonnet and Haiku (`fable`, `opus`, `sonnet`, `haiku`). An alias follows the newest model of its family, so the list does not go stale. The first entry is "Role default (<model of pm>)".
- D2. The effort list holds the levels of `claude --effort`: low, medium, high, xhigh and max. The first entry is "Role default (<effort of pm>)", or "Role default (none)".
- D3. The choice is for each product, and it changes the desk session only. The `pm` role of meetings keeps its model.
- D4. The core keeps the choice in the store. The app shows what the core sends.
- D5. A change of the model or the effort applies to the next owner message. A reply that runs keeps its process. The next message starts the process with `--resume`, so the desk conversation stays whole.

## Design

### Core

- A new store row for each product: `desk_model` and `desk_effort`, both optional text. A schema migration adds them. Read the existing migrations in `crates/agentsi-store` and follow them.
- `plan` uses the stored value when it is set, else the value of the `pm` role.
- New JSON-RPC methods in `crates/agentsi-proto/src/desk_api.rs`, also in `schema/core.openrpc.json`:
  - `desk.settings {product}` returns `{model, effort, role_model, role_effort}`. `model` and `effort` are the stored choice or null.
  - `desk.configure {product, model, effort}` stores the choice. Null clears it back to the role default.
- `desk.configure` checks each value against the lists of D1 and D2 and refuses another value with a clear message.
- When a reply runs, `desk.configure` stores the choice, and the running process stays. The process of the next owner message uses the new values. When no reply runs and the process is idle, the core ends it, so the next message starts it again with `--resume`.

### App

- `crates/agentsi-app/src/screens/desk_conversation.rs` shows the two selects in the header of the desk conversation, on one row, before the other controls.
- Use the select of `gpui-component` when it fits (`gpui_kit::component::select`). Else use a `DropdownMenu` on a `Button` (see `tab_bar.rs`). All colors and sizes come from `agentsi-ui` tokens.
- Each select shows the current value. A pick calls `desk.configure`, and the select shows the new value when the core answers. An error of the core shows as one line under the selects.
- The selects stay enabled while a reply runs. A one-line note says: "The next message uses <model>."
- The palette gets two entries: "Desk: Choose model…" and "Desk: Choose effort…". Use the second step of the palette (`PaletteStep`, `crates/agentsi-app/src/palette.rs`).

## Acceptance criteria

- [ ] The desk conversation shows a model select with "Role default (…)", Fable, Opus, Sonnet and Haiku, and an effort select with "Role default (…)", low, medium, high, xhigh and max.
- [ ] A pick of Sonnet makes the next desk process run with `--model sonnet`. A test of `plan` and `desk_args` checks the arguments.
- [ ] A pick of "Role default" clears the choice, and the next process uses the model of the `pm` role.
- [ ] A pick while a reply runs does not end that reply. The next message uses the new model.
- [ ] The next process after a change uses `--resume` with the same session id, so the conversation stays.
- [ ] The choice of a product stays after a restart of the core. A test writes it, opens the store again and reads it.
- [ ] Two products keep their own choice.
- [ ] `desk.configure` refuses a model or an effort that is not in the lists, and the store stays as it was.
- [ ] The palette entries "Desk: Choose model…" and "Desk: Choose effort…" open a second step with the same lists.
- [ ] The `pm` role of a meeting keeps its own model after a desk choice.
- [ ] `schema/core.openrpc.json` lists the two new methods, and its test passes.
- [ ] The idle app draws 0 frames with the desk conversation open.
- [ ] Owner check: the selects look right on the M1, and a Sonnet reply comes back after a pick.
