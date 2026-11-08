# /agentsi reflect

Let the agents learn from what happened. Run `agentsi reflect`, then walk the owner through the result. Talk in the owner's language, direct: the `owner.language` field that `agentsi whoami` prints.

Reflect only learns from **grounded signals**: failed checks, unresolved or invented sources, answers with no data, timeouts, the owner's marks, rejected decisions, reverted reflections. If there are no signals, there is nothing to learn. Never ask an agent, or yourself, to critique its work without signals. That rule is ADR 0001.

## 1. Check the evidence

Run `agentsi audit --product <id>` first, unless the owner just did.

- "Listo para reflexionar" lists agents: go on.
- "no hay evidencia suficiente": say so. Offer `agentsi reflect --memory-only`, which curates memory without a prompt proposal. Do not push the owner.

## 2. Run it

Run `agentsi reflect --product <id>`. For a quick memory cleanup that never proposes a prompt change, add `--memory-only`.

It calls a model, so it costs money, a little for memory (a small model) and more for a proposal (a larger model plus a replay). Tell the owner that before you run it, and wait for the owner's OK.

It takes a few minutes. Run it with Bash in the background (`run_in_background: true`) and do not poll.

## 3. Explain the result

Say it in plain words, short sentences, one topic each.

**Signals.** Which meetings and marks it looked at, grouped by agent. Example: "QA cited three paths that do not exist, in two meetings."

**Memory changes.** Per agent:
- items added, with the evidence (the minutes id) and whether each is a `fact` or a `lesson`;
- items retired, with the reason;
- items marked helpful or harmful.

A **lesson** comes from a failure or a mark, not from the repo. Show the diff of the rendered items. Say that every change cites a signal, and that changes without a signal were dropped.

Memory changes are already applied and committed. The owner can undo them (section 6).

**Prompt proposal.** Only agents whose evidence crossed a threshold get one: the same failure class in 3 meetings, or 3 rejected priorities. An agent's evidence counts from its last proposal. One reflection proposes for one agent, the one with the strongest evidence. The output lists every agent past the threshold with one of these outcomes:
- "evidencia insuficiente para proponer": below 4 replay cases, or fewer than 2 of them held out. No proposal; the evidence waits.
- "queda para la próxima reflexión": another agent got this reflection's proposal. Its evidence waits for the next run.
- "la propuesta falló": the call failed. Show the error.
- "propuesta", with two possible results:
  - `descartada`: the replay showed a worse number, went over the 15 USD budget, or repeated a change the owner already rejected. Show the numbers and say nothing was changed.
  - A pending decision: go to section 4.

## 4. A pending proposal

Show:
1. The target (the overlay of `<agent>` in product `<p>`, stored in the database and named `workspace/<p>/agents/<agent>.md`, or the file `plugin/agents/<role>.md` when the same lesson holds in two or more products).
2. The diff of the prompt.
3. The **replay table**: per held-out case and totals, before versus after, for first-pass check rate, unresolved sources of the agent, invented sources, missing objectives and cost.
4. A verdict in one sentence: no metric got worse and at least one improved, or why you doubt it. Say how many held-out cases back the claim. With few cases, say the evidence is thin.

Then ask what the owner wants. Do not decide for the owner. The owner approves in the front desk. When the owner says "approve <id>", run `agentsi decide <decision-id> approve` (or `reject`). Run it only after the owner says so for that decision. The Memory screen of the desktop app (`Ctrl+Shift+Y`) also shows the proposal. A click on Approve or Reject there is the owner's word.

Approving applies the stored diff and commits. If the target file changed since the proposal, approval refuses and marks the proposal stale; say that, and offer to run reflect again. Rejecting records it, and reflect will not propose the same diff again.

## 5. Close

In at most 5 lines: what it learned, what it changed, what waits for the owner's decision, the commit, and the cost.

## 6. Revert

If the owner dislikes a memory change, run `agentsi reflect revert <reflection-id>` after the owner's OK. It applies the inverse of that entry's memory changes, bumps `harmful` on the items it added, records the revert, and commits. A reverted reflection counts as a signal next time. The Memory screen of the desktop app has a Revert button next to the entry. Revert shows what it will change, and a click on Confirm writes it. That click is the owner's word.

To revert an approved prompt change, revert its approval commit with `git revert <commit>` (it touches the target file and `reflections.jsonl`). For an overlay, `agentsi reset <agent>` also works, but it removes the whole overlay, setup's part included. Confirm first.

## Limits

- Reflect never edits `packages/`, `apps/`, `plugin/skills/`, `scripts/` or any check code. If it asks to, stop and tell the owner.
- Never approve a decision for the owner. A reflect output is not the owner's approval.
- If `agentsi reflect` fails, show the error and the command. Do not edit memory or overlay files by hand.
