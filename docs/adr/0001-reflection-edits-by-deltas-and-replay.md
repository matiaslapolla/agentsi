# Reflection edits memory by deltas and prompts only through replay and approval

Reflection lets an LLM propose memory changes only as deltas: add an item with evidence, retire an item by id, mark an item helpful or harmful. Plain code validates and applies them, and no LLM ever rewrites a whole memory file. Prompt changes are never applied automatically. A prompt change must improve one held-out replay metric without worsening any, and then the owner must approve it.

The research (`docs/research/self-improving-agents.md`) showed why:
- A whole-context rewrite collapsed ACE's context from 18,282 tokens to 122, and accuracy fell below the baseline.
- Self-correction without external feedback degrades results (Huang et al.).
- AgentSI meets about its own code, so a self-edit loop without a fixed gate could weaken the check that judges it.

## Considered options

- Automatic memory curation after every meeting. Rejected: the owner wants reflection only on demand.
- Optimizer libraries (DSPy, GEPA). Rejected: they need 100+ labeled examples, and AgentSI has 2 meetings.
