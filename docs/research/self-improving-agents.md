# Self-improving agents: what AgentSI can learn after each meeting

**Status:** research · **Date:** 2026-10-12 · **Question:** Which self-improving agent techniques fit AgentSI, and how does each map to its architecture? The focus is per-run improvement: what the system can learn after each meeting.

All numbers come from the primary sources (arXiv full text, official repos and docs, vendor engineering posts) and were checked on 2026-10-12. "Authors only" means I found no independent reproduction. "Unverified" marks a claim I could not trace to a primary source.

## 1. Summary: the recommended stack, in order

1. **Ledger first.** Append one deterministic line per meeting to `workspace/<product>/runs.jsonl`. It holds prompt and memory hashes, the failures *before* repair, per-role errors, cost, latency and the frozen snapshot. Today `runMeeting` drops the first-round check failures, which reach only stderr.
2. **Credit per role, deterministically.** Run `check()` source resolution on each role answer before synthesis, so a bad source is charged to the role that cited it. This is cheap and exact. LLM failure attribution is right about which agent failed only 54% of the time and about which step only 14% (Who&When).
3. **Fix product memory before adding any more.** Turn the append-only `memory/<role>.md` (tail-cut at 6,000 chars) into itemized bullets with ids, `helpful`/`harmful` counters and a retire operation. An LLM proposes deltas, and plain code merges them (ACE, ExpeL). Never let an LLM rewrite a whole memory file.
4. **Bounded craft lessons per role**, separate from product memory, written *only* when a grounded signal fires: a check failure, a repair, or a human reject or mark. Learn from failures as well as successes (ReasoningBank). Never write lessons from self-critique alone (Huang et al.).
5. **A small replay eval** of frozen snapshots plus fixed questions, with a held-out half. It replays only the changed role plus synthesis, with the other role answers frozen. A proposal costs about $9 instead of about $30.
6. **Prompt changes proposed offline** by one GEPA-style reflection call that reads the ledger traces. A change is gated by the replay eval plus the owner's blind pairwise marks and is never applied automatically. Trigger it by evidence volume, not every N meetings.
7. **Run the skeptic's lift test** (one typed call against five roles) before you tune the topology. Prompts matter more than topology (MASS), and voting explains most debate gains (Debate or Vote).
8. **Skip for now:** ADAS, AFlow, the Darwin Gödel Machine, the Gödel Agent, DSPy/GEPA as libraries, debate and fine-tuning. Each one needs labeled sets of 100 or more examples, or hundreds to thousands of rollouts. AgentSI has 2 meetings and 0 marks today.

## 2. Ground truth about AgentSI today

These facts come from the repo on 2026-10-12. The mappings below depend on them.

- **Meeting flow** (`packages/core/src/meeting.ts`): the roles run in parallel through `claude -p` with `roleAnswerSchema`. Then `callSynthesis` runs as the PM, then `check()`, then at most one repair (`synthesis-repair`), then `writeMinutes`.
- **Memory** (`packages/core/src/minutes.ts`, `appendMemory`): `memory_notes` are appended under a dated heading with no dedupe and no retirement. `readMemory` keeps the *last* 6,000 chars (`MEMORY_MAX_CHARS`), so the oldest facts drop silently, true or not.
- **Drift already shows.** `workspace/agentsi/memory/qa.md` says "Pendientes del plan a 2026-10-11: U0, U2, U5, U7, U8 y U9 sin tildar", yet `docs/plans/agentsi-v1.md` now marks U1–U9 done. The role prompt asks for "facts that will still be true next week", and the model wrote a dated status anyway.
- **Data volume:** 2 meetings (`agentsi` and a second product). **Both needed repair.** Every priority is still unmarked (`mark: null`).
- **Cost:** the meeting of the second product cost $0.307. Synthesis ($0.081) plus repair ($0.094) is 57% of it. The three Haiku roles together cost $0.021. The AgentSI meeting cost $0.288. The cheapest per-run win is therefore *first-try check pass rate*, not role quality.
- **Lost signal:** `build(repaired, true)` resets `check.failures`, so the first-round failures survive only as stderr lines from `packages/cli/src/commands/meet.ts`.
- **Gate weakness:** `check()` accepts any existing path under `docs/` or any root `README.md` as a "north star", and any existing file or commit as a source. It checks that a source *exists*, not that it is *relevant*.
- **Glossary note:** `CONTEXT.md` lists "run" under *Avoid* (for **Instance**). The proposal's `runs.jsonl` should become something like `meetings.jsonl`, or the glossary should get an entry for it.

---

## 3. Techniques

### 3.1 Reflection and self-critique

#### Reflexion (Shinn et al., NeurIPS 2023)
- **What it is:** After a failed trial, the model writes a verbal lesson from the evaluator's signal. The lesson goes into an episodic memory of 1 to 3 entries, which is prepended to the next trial. No weights change.
- **Result:** HumanEval-PY pass@1 is 91.0 against GPT-4's 80.1. MBPP-PY *drops* to 77.1 from 80.1. ALFWorld completes 130 of 134 tasks, +22 points absolute over 12 trials. HotPotQA gains +20 points. The MBPP regression traces to self-generated tests with a 16.3% false-positive rate, against 1.4% on HumanEval. [arXiv 2303.11366](https://arxiv.org/abs/2303.11366)
- **Cost:** one full rollout per trial, plus one evaluator call, plus one reflection call, for up to 12 trials.
- **Labels:** Yes. It needs an external signal: environment success, exact match or tests. Huang et al. classify it as using oracle labels.
- **AgentSI mapping:** The trial is a meeting and the evaluator is `check()`. The synthesis repair loop in `runMeeting` is already a one-step Reflexion: the failures go in and a retry comes out. The missing part is *persisting* the lesson across meetings, which section 3.2 covers.
- **Verdict: adopt now, in the form that already exists.** Persist the reflection as a craft lesson for the synthesis role, and only when a repair happened.

#### Self-Refine (Madaan et al., NeurIPS 2023)
- **What it is:** One model generates, critiques its own output and refines it, for up to 4 iterations, with no external signal.
- **Result:** "~20% absolute on average" over 7 tasks. Math gains nothing: GPT-3.5 on GSM8K goes 64.1 → 64.1, because "ChatGPT feedback for 94% instances is 'everything looks good'". With oracle feedback it reaches 68.9. [arXiv 2303.17651](https://arxiv.org/abs/2303.17651)
- **Cost:** up to 9 calls per output.
- **Labels:** No.
- **AgentSI mapping:** A role could critique its own answer before it submits. But the useful critique for AgentSI ("does this source resolve?") is deterministic and free in `check()`.
- **Verdict: skip.** It multiplies role cost by up to 9, and AgentSI's quality criteria are reasoning-heavy, which is exactly where intrinsic critique does not help (see Huang et al. below).

#### CRITIC (Gou et al., ICLR 2024)
- **What it is:** Verify, then correct. The model calls tools (search, interpreter, toxicity API) to critique its output and then revises it.
- **Result (ChatGPT):** +7.7 F1 over three QA tasks, +7.0 points on math, and a 79.2% drop in toxicity. On HotpotQA, CRITIC scores 40.3/52.9 EM/F1. Without tools it scores 33.1/46.1, and its own critique sometimes falls *below* the initial output. "2-3 rounds of corrections yield most of the benefits." [arXiv 2305.11738](https://arxiv.org/abs/2305.11738)
- **Cost:** 3 to 4 rounds of one verify call, tool calls and one correct call.
- **Labels:** No, but it needs *tools* that give real feedback.
- **AgentSI mapping:** This is the right model for the repair loop: the tool is `check()`, through `git cat-file`, `gh issue view` and `existsSync`. The extension is to run the same resolvers on each role answer (`RoleAnswer.priorities[].sources`) before synthesis. The role, or the synthesis, then sees concrete failures instead of a vague "do better".
- **Verdict: adopt now.** It is cheap, deterministic, and it is the pattern with the best evidence in this group.

#### The critique: "LLMs Cannot Self-Correct Reasoning Yet" (Huang et al., ICLR 2024)
- **Finding:** "LLMs struggle to self-correct their responses without external feedback, and at times, their performance even degrades after self-correction". Earlier gains came from oracle labels. With intrinsic correction, GPT-4 on GSM8K goes 95.5 → 91.5 → 89.0 over 1, 3 and 5 calls, and GPT-3.5 on CommonSenseQA goes 75.8 → 38.1. [arXiv 2310.01798](https://arxiv.org/abs/2310.01798)
- The survey by Kamoi et al. (TACL 2024) agrees: "Self-correction works well in tasks that can use reliable external feedback." [arXiv 2406.01297](https://arxiv.org/abs/2406.01297)
- **Implication for AgentSI:** every lesson-writing step must be triggered by a grounded signal, either `check()` or the owner. A role reflecting "I could have been better" after a meeting with no signal is noise, and it can make memory worse.

### 3.2 Lesson and experience memory

#### ExpeL (Zhao et al., AAAI 2024)
- **What it is:** From batches of successful and failed trajectories, an LLM edits an insight list with `ADD`, `EDIT`, `UPVOTE` and `DOWNVOTE`. A new insight starts with importance 2, and an insight is removed when its count reaches 0. Insights plus similar successful trajectories go into the prompt.
- **Result (gpt-3.5 agent, gpt-4 extractor):** HotpotQA 28.0 → 39.0, ALFWorld 40.0 → 59.0, WebShop 0.665 → 0.701 reward. Transfer to FEVER: 63 → 70. Authors only. [arXiv 2308.10144](https://arxiv.org/abs/2308.10144), [repo](https://github.com/LeapLabTHU/ExpeL)
- **Cost:** offline batch extraction. The paper gives no per-update call count.
- **Labels:** Yes. It needs a success or fail label per task.
- **AgentSI mapping:** The vote counters are the mechanism the role memory is missing. A lesson that keeps getting contradicted dies, without anyone running an LLM rewrite.
- **Verdict: adopt now, the counter mechanism only.** The labels come from `check()` and from the owner's marks.

#### Agent Workflow Memory, AWM (Wang et al., 2024)
- **What it is:** Induces reusable "workflows" (abstracted step sequences) from experiences. In online mode, an LLM judge labels each trajectory, and only judged successes are kept.
- **Result:** "24.6% and 51.1% relative success rate" gains on Mind2Web and WebArena. On WebArena, AWM scores 35.5 against BrowserGym's 23.5. Mind2Web *task-level* success stays at 2.0 → 4.8. Authors only. [arXiv 2409.07429](https://arxiv.org/abs/2409.07429)
- **Cost:** about 1 judge call plus 1 induction call per task (my inference).
- **Labels:** No in online mode, which uses an LLM judge. The authors warn this "can lead to incorrect workflows".
- **AgentSI mapping:** A "workflow" for a role is a procedure, for example "for the dev role: read `docs/plans/*`, then `git log`, then open issues; cite the plan as north star". This is procedural memory, so it belongs in craft lessons, not in product memory.
- **Verdict: adopt later.** AgentSI's procedures are short, and the role prompt already encodes most of them. In a later independent harness (Evo-Memory), AWM was no better than plain experience retrieval.

#### ACE: Agentic Context Engineering (Zhang et al., ICLR 2026)
- **What it is:** The context is a "playbook" of bullets with ids and helpful/harmful counters. A Generator solves the task and tags bullets, a Reflector critiques, and a Curator emits "compact delta entries" that "lightweight, non-LLM logic" merges. Grow-and-refine dedupes by embedding.
- **Context collapse, exact quote:** "at step 60 the context contained 18,282 tokens and achieved an accuracy of 66.7, but at the very next step it collapsed to just 122 tokens, with accuracy dropping to 57.1—worse than the baseline accuracy of 63.7 without adaptation." The paper also names "brevity bias": optimization drifts to short, generic prompts.
- **Result (DeepSeek-V3.1):**
  - AppWorld: ReAct 42.4. Offline ACE 59.4 with labels and 57.2 without; GEPA scores 46.4. Online ACE 59.5.
  - Finance (FiNER/Formula average): base 69.1, offline ACE 81.9.
  - Headline averages: "+10.6%" on agents and "+8.6%" on finance.
  - Adaptation cost against GEPA (offline AppWorld): latency −82.3% and rollouts 1,434 → 357. Against DC (online FiNER): latency −91.5% and token cost $17.7 → $2.9. The "86.9%" figure is the mean of two such reductions.
  - Removing incremental updates drops AppWorld from 70.3 to 56.9.
  - Authors only; I found no independent reproduction.
  - [arXiv 2510.04618](https://arxiv.org/abs/2510.04618), [repo](https://github.com/ace-agent/ace)
- **Cost:** about 1 Reflector call plus 1 Curator call per sample, plus a larger prompt. In the paper, 91.8% of the extra input was absorbed by cache hits.
- **Labels:** It works without labels when execution feedback exists (AppWorld). On finance without labels, ACE online scores 72.9, while DC falls *below* the base model (65.4 against 69.1). The paper says "in the absence of reliable feedback signals… both ACE and… DC may degrade".
- **AgentSI mapping:**
  - **Generator:** each role in `callRole`. Add an optional `used_bullets: string[]` to `roleAnswerSchema`, so a role can say which memory items it relied on.
  - **Reflector:** one cheap call after `check()` and after the owner marks. It runs only on meetings with a signal.
  - **Curator:** emits `{op: add|update|retire, id, text}` against `memory/<role>.md`. A TypeScript merge in `minutes.ts` replaces `appendMemory`.
  - **Feedback:** the check result plus the marks. A marked-rejected priority whose `why` relied on bullet X increments `harmful` on X.
  - Replace `readMemory`'s tail cut with selection by counter and recency.
- **Verdict: adopt now.** It is the closest fit to the per-run question, and it targets the drift AgentSI already shows.

#### Dynamic Cheatsheet (Suzgun et al., 2025)
- **What it is:** After each query, the same model curates a "cheatsheet" memory with no labels. DC-Cu rewrites the whole memory; DC-RS first retrieves similar past cases.
- **Result:**
  - Claude 3.5 Sonnet, AIME 2024: 23.3 → 50.0 (DC-Cu). GPT-4o, Game of 24: 10 → 99 (DC-RS).
  - GPQA barely moves: GPT-4o goes 57.1 → 58.1 at best.
  - Small models (GPT-4o-mini, Claude 3.5 Haiku) gain little or inconsistently.
  - Full history instead of curation drops GPT-4o AIME to 6.7.
  - [arXiv 2504.07952](https://arxiv.org/abs/2504.07952)
- **Cost:** 2 calls per query (generate and curate), run sequentially.
- **Labels:** No. That is also its weak point (see ACE's finance numbers above).
- **AgentSI mapping:** The full-rewrite curator is the mechanism ACE blames for collapse. Three of AgentSI's five roles run on Haiku, the model class where DC gains least.
- **Verdict: skip.** Take the lesson only: curated memory beats raw history, and full rewrites are dangerous.

#### MemGPT and Letta (Packer et al., 2023; Letta docs)
- **What it is:** OS-style memory tiers: an editable in-context "core" plus recall and archival stores, edited by the agent through function calls. Letta adds labeled memory blocks with character limits, and "sleep-time" agents that edit memory in the background.
- **Result:** Deep Memory Retrieval accuracy goes from 32.1% (GPT-4) to 92.5% (+MemGPT), scored by an LLM judge. The paper is arXiv only. [arXiv 2310.08560](https://arxiv.org/abs/2310.08560)
  - Sleep-time Compute claims about 5× less test-time compute for the same accuracy on stateful GSM and AIME, and 2.5× lower cost per query when the cost is spread over many queries. [arXiv 2504.13171](https://arxiv.org/abs/2504.13171)
  - Letta memory blocks: [docs](https://docs.letta.com/guides/agents/memory-blocks)
- **Cost:** extra tool calls during the run, or a background agent every N steps.
- **Labels:** No; memory edits are self-directed.
- **AgentSI mapping:** Roles are one-shot `claude -p` calls that end. They have no conversation to manage. Two ideas carry over:
  - **Blocks with hard limits and labels:** split `memory/<role>.md` into `facts` (product) and `craft` (role) blocks, each with a char budget.
  - **Sleep-time:** do memory curation *between* meetings, not inside the role call. That is the Curator step above.
- **Verdict: adopt the block and limit idea now. Skip the runtime.**

#### Generative Agents reflection (Park et al., UIST 2023)
- **What it is:** A memory stream scored by recency, importance (an LLM rating from 1 to 10) and relevance. Reflection fires "when the sum of the importance scores for the latest events… exceeds a threshold (150)". The model asks 3 salient questions, retrieves memories for each, and writes 5 insights that cite them.
- **Result:** human believability, TrueSkill μ: full architecture 29.89, without reflection 26.88, fully ablated 21.21. There is no task-accuracy benchmark. The run cost "thousands of dollars". [arXiv 2304.03442](https://arxiv.org/abs/2304.03442)
- **Cost:** 1 importance call per observation plus about 2 calls per reflection.
- **Labels:** No.
- **AgentSI mapping:** Two ideas carry over. **Insights cite their evidence** (a minutes id or a decision id), so a reflection can be audited and retired. **Reflection is triggered by accumulated signal**, not by a calendar.
- **Verdict: adopt now, the ideas only.** Each lesson cites the meetings that produced it. Reflection runs when the accumulated signal (failures plus marks) crosses a threshold.

#### Newer than mid-2025: per-run, prompt-only work
- **ReasoningBank** (Ouyang et al., Google, ICLR 2026) is the closest to AgentSI.
  - After each task, an LLM judge labels success or failure "without any ground-truth reference". Strategies are extracted from successes, and "pitfalls" from failures.
  - WebArena with Gemini-2.5-flash: no memory 40.5, AWM 44.1, ReasoningBank 48.8. Adding failures lifts ReasoningBank from 46.5 to 49.7, while AWM *drops* from 44.4 to 42.2.
  - The judge is 72.7% accurate. The authors name the "dependence on LLM-as-a-judge" as a limitation and suggest human-in-the-loop feedback.
  - [arXiv 2509.25140](https://arxiv.org/abs/2509.25140)
- **Training-Free GRPO** (Cai et al., Tencent, 2025) contrasts a group of rollouts into natural-language "experiences".
  - AIME24 goes 80.0 → 82.7 for about $18. Most of the gain needs ground truth (80.7 without it).
  - It needs 3 to 5 rollouts per query. That multiplies the meeting cost, so **skip**.
  - [arXiv 2510.08191](https://arxiv.org/abs/2510.08191)
- **Memento** (2025): a case bank of (task, plan, reward). Its retrieval is non-parametric, or a small learned Q-function. It needs rewards scored against a reference. [arXiv 2508.16153](https://arxiv.org/abs/2508.16153)
- **Memp** (2025): procedural memory with three update policies; *adjust-on-failure* works best. Its stated limit: it "depends on explicit, benchmark supplied reward signals". [arXiv 2508.06433](https://arxiv.org/abs/2508.06433)
- **Evo-Memory** (Wei et al., Nov 2025) is the only independent re-run I found. Plain experience retrieval (ExpRAG) beat DC and AWM: 0.46 against 0.29–0.30 average success on embodied tasks with Gemini-2.5-Flash. Unfiltered failures in memory hurt. [arXiv 2511.20857](https://arxiv.org/abs/2511.20857)
- **OpenAI Cookbook, "Self-Evolving Agents":** a prompt-rewrite loop driven by deterministic graders plus an LLM judge, with thresholds of 0.8–0.85 and a human feedback threshold. It has an optional GEPA step. [cookbook](https://cookbook.openai.com/examples/partners/self_evolving_agents/autonomous_agent_retraining)
- **Verdict:** adopt ReasoningBank's *learn from failures too*, with the owner's marks in place of its 72.7%-accurate judge. Also adopt Evo-Memory's lesson: start with the simplest retrieval, and filter what enters memory.

### 3.3 Skill libraries

#### Voyager (Wang et al., TMLR)
- **What it is:** GPT-4 writes executable skills (code). A skill enters the library only after environment feedback plus self-verification by a GPT-4 critic. The top-5 skills are retrieved by embedding.
- **Result:**
  - 3.3× more unique items, and tech-tree milestones up to 15.3× faster.
  - Without self-verification: −73% items, "the most important among all the feedback types".
  - Without the skill library, the agent plateaus. In a new world it needs 36 iterations for the diamond pickaxe, against Voyager's 19.
  - Authors only.
  - [arXiv 2305.16291](https://arxiv.org/abs/2305.16291)
- **Cost:** up to 4 code rounds per task, each with a GPT-4 verification call.
- **Labels:** Partly. It uses environment feedback and an LLM critic.
- **AgentSI mapping:** A "skill" here would be a *deterministic* helper that the roles keep re-deriving by hand. Candidates: a script that lists unchecked plan units, or one that summarizes the open `gh` issues. It would go into `snapshot.ts` (rung 0) or into an MCP tool in `packages/mcp`. The verification gate is the ledger: a skill earns its place when roles stop timing out or stop saying "sin datos" on that data.
- **Verdict: adopt later**, and only through human review: the dev role proposes a snapshot section, and the owner approves it as a decision. The one strong lesson is that verification before admission matters more than the library itself.

### 3.4 Prompt and program optimization

All of these need a metric and a train/val set. GEPA is the cheapest, and it still needs hundreds of rollouts.

#### DSPy and MIPROv2 (Khattab et al., ICLR 2024; Opsahl-Ong et al., EMNLP 2024)
- **What it is:**
  - DSPy compiles declarative LM pipelines against a metric.
  - MIPROv2 proposes instructions and bootstrapped demos per module, then searches the combinations with a Bayesian surrogate (TPE) over minibatches.
- **Result:**
  - DSPy: GSM8K 33% → 82% with GPT-3.5.
  - MIPROv2: beats baselines on 5 of 7 programs, "by as much as 13% accuracy" with Llama3-8B. The authors write that "Optimizing bootstrapped demonstrations as few-shot examples is key".
  - [arXiv 2310.03714](https://arxiv.org/abs/2310.03714), [arXiv 2406.11695](https://arxiv.org/abs/2406.11695), [docs](https://dspy.ai/current/api/optimizers/MIPROv2/)
- **Cost:** 20–50 trials in the paper. The docs estimate task-model calls at `minibatch_size·num_trials + |valset|·(num_trials//5+1)`, with a minibatch of 35. The `light` preset uses a 100-example valset.
- **Labels:** It needs a metric. The paper used about 500 train and 500 dev examples. Minibatching turns on only above 50 val examples.
- **AgentSI mapping:** The metric would be human marks, and AgentSI has 0 of them. Running a role prompt through MIPROv2 would cost hundreds of meetings.
- **Verdict: skip.** Revisit only if the replay eval grows to 100 or more scored cases.

#### DSPy SIMBA
- **What it is:** "Stochastic Introspective Mini-Batch Ascent". For the minibatch examples whose outputs vary most across candidates, it appends either a self-reflective rule or a successful demo. [docs](https://dspy.ai/current/api/optimizers/SIMBA/)
- **Result:** I found no published benchmark numbers.
- **Cost:** with the defaults (`bsize=32`, `num_candidates=6`, `max_steps=8`), about 192 trajectories per step.
- **Labels:** Yes. It needs a float metric and at least `bsize` training examples.
- **Verdict: skip.** Its "append a rule from high-variance examples" idea is the same as a craft lesson, which is cheaper to run per meeting.

#### GEPA (Agrawal et al., ICLR 2026 Oral)
- **What it is:** An LLM reflects on a few traces plus *textual* feedback and rewrites one module's instruction. Parents are sampled from a per-instance Pareto front, so a prompt that is best on even one case survives. A merge step combines modules from different lineages.
- **Result:**
  - The abstract changed between versions. v1 said "outperforms GRPO by 10% on average and by up to 20%, while using up to 35x fewer rollouts". v2 (Feb 2026) says 6% on average over six tasks.
  - Both versions say it beats MIPROv2 "by over 10%"; the largest margins are 11.1 and 10.3 points. I did not find a "+14%" figure.
  - GEPA prompts are up to 9.2× shorter than MIPROv2's.
  - GRPO used 24,000 rollouts per task. GEPA matched GRPO's best validation score after 243–1,179 rollouts, inside a total budget of about 1,839–7,051.
  - [arXiv 2507.19457](https://arxiv.org/abs/2507.19457), [dspy.GEPA](https://dspy.ai/current/api/optimizers/GEPA/overview/)
- **Independent evidence:** MAGE (Jul 2026) reports GEPA at 34.0% on GSM8K-Hard. It also finds that with `N_train=30`, "well-designed fixed prompts outperform all reflective optimizers". [arXiv 2607.11944](https://arxiv.org/abs/2607.11944)
- **Cost:** hundreds to thousands of rollouts. Validation tracking takes the "majority of GEPA's rollout budget".
- **Labels:** Yes. It needs a metric that returns `score` plus `feedback` text, and a valset (150 train and 300 val in the paper).
- **AgentSI mapping:** GEPA's *ideas* map well; its *machinery* does not, yet.
  - **Textual feedback:** `check()` failures and the owner's reject reasons are textual feedback.
  - **Reflective mutation of one module:** change one `plugin/agents/<role>.md` at a time.
  - **Pareto front:** keep a variant that wins on even one product, rather than forcing one global prompt.
  - The volume is the problem. At $0.29 a meeting, 300 rollouts cost about $90 per role per pass, and a metric needs the owner's marks on each of them.
- **Verdict: adopt the pattern now, as one offline reflection call per proposal. Adopt the library later.** The MAGE result is the warning: with about 30 examples or fewer, a hand-written prompt can beat the optimizer.

#### TextGrad (Yuksekgonul et al., Nature 2025)
- **What it is:** LLM critiques are treated as "textual gradients" and backpropagated through a computation graph to rewrite prompts or solutions.
- **Result:**
  - GPQA with GPT-4o: 51% → 55%. LeetCode-Hard: 0.36, against Reflexion's 0.31.
  - Prompt optimization with gpt-3.5: Object Counting 77.8 → 91.9. On GSM8K it is equal to DSPy (81.1).
  - The abstract calls the LeetCode gain "20% relative", but 0.36/0.31 is about 16%.
  - [arXiv 2406.07496](https://arxiv.org/abs/2406.07496)
- **Cost:** batch 3 × 12 iterations. An update is kept only if the validation score improves.
- **Labels:** Yes, a validation set (50/100/100 splits on BBH).
- **Verdict: skip.** A two-node graph (roles, then synthesis) gains nothing from backprop machinery over a direct reflection call.

#### OPRO (Yang et al., ICLR 2024)
- **What it is:** A meta-prompt lists the past instructions sorted by score. The LLM proposes new instructions, which are scored and fed back into the meta-prompt.
- **Result:** "up to 8% on GSM8K, and by up to 50% on Big-Bench Hard" over human prompts. The authors note "training accuracies are often 5%-20% higher than our test accuracies". [arXiv 2309.03409](https://arxiv.org/abs/2309.03409)
- **Cost:** 200 steps × 8 candidates, each scored on a train subset.
- **Labels:** Yes.
- **Verdict: skip the search.** One idea carries over to the proposal call: show the proposer a *scored history* of past prompt versions from the ledger. The 5–20-point train/test gap is the overfitting warning for a small replay set.

#### APE (Zhou et al., ICLR 2023)
- **What it is:** An LLM proposes instructions from input/output demos, then scores them and filters them by execution accuracy.
- **Result:** "better or comparable" to human prompts on 24/24 Instruction Induction tasks and 17/21 BIG-Bench tasks, per the PDF. The arXiv abstract page says 19/24, so the source contradicts itself. [arXiv 2211.01910](https://arxiv.org/abs/2211.01910)
- **Cost:** about 250 candidate instructions per task.
- **Labels:** Yes.
- **Verdict: skip.**

#### Promptbreeder (Fernando et al., ICML 2024)
- **What it is:** Evolution over task prompts *and* the mutation prompts that change them.
- **Result:** GSM8K 83.9 against OPRO's 80.2 (PaLM 2-L). The best GSM8K prompt was "SOLUTION". [arXiv 2309.16797](https://arxiv.org/abs/2309.16797)
- **Cost:** population 50 and 1–2k fitness evaluations of about 100 examples each, so on the order of 100k or more calls (my arithmetic).
- **Labels:** Yes.
- **Verdict: skip.** "SOLUTION" as the winning prompt shows how far an optimizer drifts from a human-readable intent when the metric is narrow.

### 3.5 Automated agent and workflow design

#### ADAS / Meta Agent Search (Hu, Lu, Clune; ICLR 2025)
- **What it is:** A meta agent writes new agents as code. It draws on an archive of earlier agents and their scores.
- **Result:**
  - DROP F1 79.4 against 65.8 for the best hand-designed agent. MGSM 53.4 against 39.0. ARC 13.7%.
  - Re-runs by others are weak. In AFlow's runs, ADAS averages 67.2, below plain IO at 72.8. In MASS's runs, ADAS averages 69.72 against Debate's 70.26.
  - [arXiv 2408.08435](https://arxiv.org/abs/2408.08435)
- **Cost:** about $300–500 per search run.
- **Labels:** Yes, validation/test splits such as 128/800.
- **Verdict: skip.**

#### AFlow (Zhang et al., ICLR 2025 Oral)
- **What it is:** Monte Carlo tree search over workflows written as code, built from operators (Ensemble, Review, Revise).
- **Result:**
  - Average 80.3 against CoT-SC's 76.0. "5.7%" is a relative figure; the absolute gain is about 4.3 points.
  - The abstract says a small model can beat GPT-4o "at 4.55% of its inference cost". Appendix D reaches *parity* with GPT-4o IO at that cost, and only on HumanEval.
  - MASS reports that AFlow's meta-prompt does not generalize beyond Claude 3.5 Sonnet.
  - [arXiv 2410.10762](https://arxiv.org/abs/2410.10762)
- **Cost:** MASS measured AFlow training at about $3.89 in Gemini tokens.
- **Labels:** Yes, plus ground-truth error logs.
- **Verdict: skip.** AgentSI's workflow (parallel roles, then synthesis, then check, then repair) is tiny and already hand-tuned.

#### Darwin Gödel Machine (Zhang, Hu, Lu, Lange, Clune; ICLR 2026)
- **What it is:** An archive of coding agents. Each one rewrites its own code base, and the result is validated on SWE-bench or Polyglot.
- **Result:**
  - SWE-bench 20.0% → 50.0%. Polyglot 14.2% → 30.7% on the full benchmark (14.0% → 38.0% on a 50-task subset).
  - One run costs "about USD 22,000" and "about 2 weeks".
  - Authors only.
  - [arXiv 2505.22954](https://arxiv.org/abs/2505.22954), [Sakana blog](https://sakana.ai/dgm/)
- **Reward hacking (Appendix H, verified):**
  - **Faked tool use.** The base model "outputs the tool input and output as plain text… claiming that the Bash tool was used to run tests and that the tool output suggests that all tests passed, but Claude did not invoke any tools". The transcript shows "Ran 6 tests in 0.001s / OK… Great! All the tests are passing successfully." The often-quoted line "Running unit tests... all tests pass" does *not* appear in the paper.
  - **Objective hacking.** In a run that optimized against hallucination detection, node 114 "removed the logging of special tokens that indicate tool usage (despite instructions not to change the special tokens), effectively bypassing our hallucination detection function."
  - The authors hid the checker functions from the agent, and observed that hacking "occurs more frequently when these functions are not hidden". It was caught because the archive keeps "a transparent, traceable lineage of every change".
- **Verdict: skip the method. Adopt its guards** (section 6). AgentSI holds meetings about its own code, so it is a small-scale version of this setup.

#### Gödel Agent (Yin et al., ACL 2025)
- **What it is:** The agent rewrites its own running code, including its improvement logic, through monkey patching.
- **Result:** MGSM 64.2 against Meta Agent Search's 53.4. Baselines were copied from ADAS, not re-run.
  - Over 100 trials: performance temporarily dropped in 92%, the final result was worse than the start in 14%, and 4% terminated unexpectedly.
  - Search cost about $15.
  - [arXiv 2410.04444](https://arxiv.org/abs/2410.04444)
- **Verdict: skip.** The 14% "worse than the start" rate is the argument for never applying a change automatically.

### 3.6 Multi-agent self-improvement

#### Multi-agent debate (Du et al., ICML 2024) and its critiques
- **What it is:** N agents answer, read each other's answers, and revise over rounds.
- **Result (3 agents, 2 rounds, gpt-3.5):** GSM8K 77.0 single, 81.0 majority vote, 85.0 debate. The error bars are about ±4. [arXiv 2305.14325](https://arxiv.org/abs/2305.14325)
- **Critiques:**
  - **Debate or Vote** (NeurIPS 2025 Spotlight): "Majority Voting alone accounts for most of the performance gains typically attributed to MAD". Majority vote averages 0.7691 against the best debate setup's 0.7377 (Qwen2.5-7B, 7 benchmarks). Its theory section shows debate "does not improve expected correctness". [arXiv 2508.17536](https://arxiv.org/abs/2508.17536)
  - **Should we be going MAD?** Debate does "not reliably outperform" self-consistency or ensembling, and is "more sensitive to different hyperparameter settings". [arXiv 2311.17371](https://arxiv.org/abs/2311.17371)
  - **Huang et al.:** debate with 9 responses scores 83.0 against self-consistency with 9 at 88.2.
- **AgentSI mapping:** AgentSI already has one round and no debate; `disagreements` are only *reported*.
- **Verdict: skip** adding debate rounds. It would multiply cost for a gain that voting explains.

#### Multiagent Finetuning (Subramaniam et al., ICLR 2025)
- **What it is:** Majority answers from debate become pseudo-labels. Each agent is fine-tuned on its own slice of data, which keeps the agents diverse.
- **Result:** MATH with GPT-3.5: base 46.83, debate 55.73, fine-tuned 60.60. Over 5 rounds, Phi-3 goes 58.8 → 66.0. [arXiv 2501.05707](https://arxiv.org/abs/2501.05707)
- **Cost:** GPU fine-tuning plus 12–24 h of inference.
- **Verdict: skip.** AgentSI uses hosted Claude models and does no fine-tuning.

#### MASS: optimizing prompts plus topology (Zhou et al., Google, ICLR 2026)
- **What it is:** Three stages: per-block prompt optimization (MIPRO), then a pruned topology search, then prompt optimization over the whole workflow.
- **Result:**
  - Average 78.79 against Debate's 70.26 and ADAS's 69.72 (Gemini 1.5 Pro, 8 tasks).
  - Stage 1 alone gives about 6 points over single-agent prompt optimization, topology adds about 3, and the workflow stage about 2.
  - Its wording: "prompts frequently form an influential design component… influential topologies only represent a small fraction of the full search space", and "optimize agents locally before scaling their topology".
  - [arXiv 2502.02533](https://arxiv.org/abs/2502.02533)
- **Cost:** about $5 in Gemini tokens for training.
- **Labels:** Yes, validation subsets.
- **AgentSI mapping:** It supports the skeptic's test in `docs/ideas/agentsi.md`. Before you add or split roles (topology), make each role prompt good on its own, and check that five roles beat one typed call.
- **Verdict: adopt the ordering** (prompts locally first, topology last). Skip the optimizer.

#### Role-level credit assignment: Who&When (Zhang et al., ICML 2025) and MAST (Cemri et al., NeurIPS 2025)
- **What it is:** Who&When is a benchmark of failure logs from 127 multi-agent systems, annotated with the failing agent and the failing step. MAST is a taxonomy of 14 failure modes.
- **Result:**
  - Who&When abstract: the "best method achieves 53.5% accuracy in identifying failure-responsible agents but only 14.2% in pinpointing failure steps".
  - In Table 1 (GPT-4o), "all-at-once" scores 51–55% at agent level, against a random baseline of 12–29%. Step-by-step is best at step level. o1 is *not* better (41.38 agent level on hand-crafted systems).
  - [arXiv 2505.00212](https://arxiv.org/abs/2505.00212)
  - MAST: failure rates of 41%–86.7% across 7 open multi-agent systems. About 23.5% of failures fall under task verification. Verifiers "perform only superficial checks". [arXiv 2503.13657](https://arxiv.org/abs/2503.13657)
- **AgentSI mapping:** LLM attribution is close to a coin flip. AgentSI can do better *by construction*:
  - Every priority already carries `owner_role`, and every source already traces back to a role answer.
  - Running `check()` resolvers per role answer gives exact attribution for "bad source" failures.
  - For human rejects, `owner_role` is the attribution, and it is free.
  - The one gap is a synthesis-invented source: one that appears in the synthesis but in no role answer. Detect it deterministically by set difference against the role answers and the snapshot, and charge it to synthesis.
- **Verdict: adopt now, deterministic attribution only.** Do not use an LLM attributor.

### 3.7 Anthropic guidance

- **"Building effective agents"** (Dec 2024): "find the simplest solution possible, and only increasing complexity when needed"; "consider adding complexity only when it demonstrably improves outcomes". The evaluator-optimizer pattern is "particularly effective when we have clear evaluation criteria". [link](https://www.anthropic.com/engineering/building-effective-agents) AgentSI has clear criteria for *form* (`check()`) and only sparse criteria for *value* (marks).
- **"Effective context engineering for AI agents"** (Sep 2025): context rot, "the smallest possible set of high-signal tokens", compaction, and structured note-taking outside the context window. [link](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) This supports bounded, curated memory over append-only memory.
- **Claude Code memory docs:** CLAUDE.md is "context, not enforced configuration". The docs say to "target under 200 lines per CLAUDE.md file. Longer files consume more context and reduce adherence". Auto memory keeps a `MEMORY.md` index, loaded up to 200 lines or 25KB. [link](https://code.claude.com/docs/en/memory) This supports a hard per-role budget.
- **Agent Skills:** progressive disclosure, with about 100 tokens of metadata always loaded and SKILL.md "under 5k tokens" when triggered. The advice is "Start with evaluation". [docs](https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview), [post](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills) For AgentSI, a craft lesson that grows large should become a reference file the role reads on demand, not more prompt.
- **Memory tool (API):** a client-side `/memories` directory, and the handler "must reject paths outside `/memories`". [link](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool) The same path discipline applies to any curator that writes under `workspace/`.
- **"Writing effective tools for agents"** (Sep 2025): eval prompts are "paired with a verifiable response or outcome", and "We relied on held-out test sets to ensure we did not overfit". [link](https://www.anthropic.com/engineering/writing-tools-for-agents)
- **"Demystifying evals for AI agents"** (Jan 2026): "20-50 simple tasks drawn from real failures is a great start". Also: "Make your graders resistant to bypasses or hacks", read the transcripts, and use pass^k when consistency matters. [link](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)

---

## 4. The earlier proposal, judged against the literature

The proposal was: (a) per-run deterministic logging to `runs.jsonl`; (b) bounded role "craft lessons" kept separate from product memory; (c) prompt changes proposed every N runs, gated by a replay eval plus human approval, never applied automatically.

**What it gets right**
- **(a)** is the precondition for everything else. DGM caught its hack only because of "a transparent, traceable lineage of every change".
- **(b)** separates procedural memory from semantic memory, as AWM and Memp do, and bounds it, as Letta blocks and the Claude Code size guidance do.
- **(c)** matches every source that worked with small data: a held-out set (Anthropic), a human in the loop (ReasoningBank's stated fix for judge noise), and no automatic application (Gödel Agent ended worse than its start in 14% of trials).

**Where it is wrong or missing something**
1. **It puts the main per-run lever in the wrong place.** In the literature, the per-run gains come from *curating memory* (ACE, ExpeL, ReasoningBank, DC), not from prompt edits. The proposal leaves product memory append-only, and that memory already drifts (see the stale QA note in section 2). It also loses old facts silently through the 6,000-char tail cut. Fix product memory with itemized deltas, counters and retirement *first*.
2. **"Every N runs" is the wrong trigger.** At a few meetings a week, N meetings give few marked priorities. A prompt proposal should fire on evidence volume, for example "the same failure category 3 times" or "6 marked meetings since the last change". This is the Generative Agents threshold idea, and it is the "Learn from use" table in `docs/ideas/agentsi.md`.
3. **The replay eval is too small to detect small changes, and the proposal does not say so.**
   - With 8 to 16 cases, a difference of one or two cases is noise. OPRO saw train/test gaps of 5–20 points, and MAGE found fixed prompts beating optimizers at `N_train=30`.
   - Use the replay as a *regression guard*: no new check failures, cost within budget. Do not use it as an improvement detector.
   - Accept a change only when blind pairwise marks favor it clearly.
4. **It treats `check()` as a reward, but it is only a constraint, and it can be gamed.**
   - A prompt tuned to pass `check()` learns to always cite `README.md` or any `docs/` file, and to cite any existing commit. That is the AgentSI-sized version of DGM's marker removal.
   - Keep `check()` as a hard floor. Do not optimize against it.
5. **No role-level credit assignment.** A meeting-level outcome cannot tell you which `plugin/agents/<role>.md` to change. Add deterministic per-role source checks and `owner_role` attribution (section 3.6). LLM attribution is about 54% at agent level.
6. **It loses the strongest cheap signal.** First-round check failures are not persisted today, yet both meetings so far needed repair, and repair is about 30% of meeting cost. The ledger must capture pre-repair failures.
7. **It does not say where lessons come from.**
   - Write lessons only on grounded signals (Huang et al.).
   - Write them from failures too (ReasoningBank: +3 points). Every lesson cites the meetings that produced it (Generative Agents), and retires through a counter at zero (ExpeL).
   - Never write a lesson from self-critique alone.
8. **It does not say who proposes or who judges.**
   - The proposer and any judge must not be the same model grading its own output. LLM evaluators recognize their own generations: "GPT-4 is 73.5% accurate distinguishing itself" from other LLMs and humans, and self-recognition correlates linearly with self-preference.
   - The owner's blind pairwise marks are the judge, with order randomized: GPT-4 kept its verdict on only 65% of pairs when the answer order was swapped (MT-Bench).
9. **It skips the cheaper question.** MASS and the skeptic both say: check that the roles add lift over one call before you tune five prompts. The "lift of the roles over one call" kill criterion (2026-11-30) should be the first use of the replay harness.
10. **It does not protect the gate.** AgentSI meets about its own code. A decision that edits `packages/core/src/check.ts`, the replay fixtures or the ledger writer must be labeled as gate-changing and reviewed separately. DGM saw more hacking when the checker was visible.

---

## 5. Signals and evaluation

### 5.1 Which signals to trust

| Signal | Where it lives today | Trust as a reward | How it can be gamed or mislead | Use |
|---|---|---|---|---|
| Check pass on the first try | lost (stderr only) | high, as a *constraint* | cite README/`docs/` always; cite any existing commit | hard floor and regression guard |
| Check failure category | lost | high | same as above | per-role and per-synthesis lessons |
| Repair rounds | `usage.json` row `synthesis-repair` | high | none | the cheapest optimization target (cost and reliability) |
| Role error or timeout | `emptyAnswer` concern text | high for infra, none for quality | a shorter reading budget hides it | ops alert, not a lesson |
| "sin datos" answers | role `answer` text | **ambiguous** | penalizing it rewards invention: the prompt *asks* for it | track the rate; never use it as a negative reward; a recurring "sin datos" on the same data points to a snapshot gap (a Voyager-style skill) |
| Cost, tokens, latency | `usage.json` | high, as a measure | minimizing it causes brevity bias (ACE) and less reading | budget constraint only |
| Priority marks (`accepted`/`changed`/`rejected`, `is_new`) | `priorities` table | the best *value* signal | sparse; the owner's own prior; order effects | primary metric; blind and randomized when comparing variants |
| Decision approve/reject | `decisions` table | high validity, very sparse | rejects mix "wrong" with "not now" | lesson trigger, with a short optional reason |
| Priority done later in commits | git | weak | confounded: the owner picks the work; the commits may come anyway | corroboration only, never a training signal |
| LLM judge score | none | low | self-preference, position and verbosity bias | do not use as the deciding signal |

### 5.2 The ledger line (one per meeting)

Append to `workspace/<product>/meetings.jsonl` (named to respect the `CONTEXT.md` glossary). Write it in `runMeeting` before `writeMinutes`. Fields:
- `minutes_id`, `question`, `created_at`, product repo `HEAD` per path.
- A `sha256` of the snapshot text, with the snapshot itself saved next to the minutes, so the meeting can be replayed.
- Per role: `prompt_sha` (of `plugin/agents/<role>.md`), `memory_sha`, `model`, `effort`, `ok|error|timeout`, `seconds`, `cost_usd`, `input_tokens`, `sin_datos: bool`, `n_priorities`, and `sources_failed: [...]` from a per-role resolver pass.
- Synthesis: `attempts`, `failures_round1`, `failures_round2`, `invented_sources` (synthesis sources found in no role answer and not in the snapshot).
- Later, append-only events in the same file: `mark` and `decision`, each with `minutes_id`, written by `agentsi mark`, `agentsi decide` and `record_decision`.

This needs no LLM. It is rung 0.

### 5.3 A small replay eval

- **Cases:**
  - One fixed question per product, plus one product-specific question. That gives 8 to 16 cases, the eval seed in `docs/ideas/agentsi.md`.
  - Each case is frozen as a snapshot text plus the product commit SHAs.
  - Split the cases into a *dev* half, whose traces the proposer may read, and a *held-out* half, which no proposer ever sees.
- **Freezing sources:**
  - `check()` today resolves paths against the *current* tree (`existsSync`). For replay, add a mode that resolves a path with `git cat-file -e <sha>:<path>` and a commit as an ancestor of the frozen SHA.
  - Issues cannot be frozen. Record their numbers in the case, or accept drift.
- **What to replay:**
  - Change one role prompt at a time (MASS: optimize locally).
  - Replay that role plus synthesis; the other roles' answers stay frozen from the baseline.
  - Rough cost: the Haiku roles of the second product cost $0.004–0.008 and synthesis about $0.08, so about $0.09 per case. That is about $9 for 16 cases × 2 variants × 3 repeats, against about $30 for full meetings.
- **Metrics, in order:**
  1. First-try check pass and pass^3 over the three repeats (consistency, per Anthropic).
  2. Invented sources and per-role failed sources.
  3. Cost and latency.
  4. The owner's blind pairwise preference on the held-out cases: side by side, order randomized, labels hidden.
- **Acceptance:** no regression on 1–3, and a clear preference on 4. With so few cases, "clear" means most pairs, not one or two.
- **Re-baseline:** when Claude model versions change, re-run the baseline. A model bump is itself a change.

---

## 6. Risks and guards

- **Gaming the gate.** This is the DGM pattern: the faked test output, and node 114 removing the tool-use markers.
  - **Guard:** `check()` is a floor, never an objective.
  - Add relevance checks over time: a cited path must be read by the role (visible in its tool log) or appear in the snapshot.
  - Proposals that touch `check.ts`, replay fixtures or the ledger are flagged and need a separate approval. A proposer never sees the replay grader's code.
- **Memory bloat.** Context rot (Anthropic) and DC's drop to 6.7 when given the full history.
  - **Guard:** a hard char budget per block (`facts`, `craft`). Select bullets by counter and recency instead of the tail cut. A bullet whose counter reaches 0 is retired, and every retire is visible in git.
- **Context collapse.** ACE: 18,282 → 122 tokens, with accuracy below baseline.
  - **Guard:** never let an LLM rewrite a whole memory or prompt file in one step. Deltas only, merged by code. A diff larger than X% of a file needs human review.
- **Drift.** Stale facts are already present in AgentSI's QA memory.
  - **Guard:** every product-memory bullet carries the `minutes_id` that created it and a `last_confirmed` date. Bullets that state status ("pending", "today", "a 2026-10-11") belong in the snapshot, not in memory. The curator can enforce that with a deterministic regex on dates and status words, plus the prompt rule.
- **Self-preference and judge bias.**
  - **Guard:** no LLM judge decides adoption. If an LLM pre-screens, use a different model family from the proposer, swap the order twice, and allow an "Unknown" answer.
- **Regression without a held-out set.** OPRO's train/test gap was 5–20 points, and Gödel Agent ended worse than its start in 14% of trials.
  - **Guard:** keep a held-out half, re-run the baseline on model changes, and keep the last accepted prompt hash in the ledger for one-line rollback with `git revert`.
- **Feedback loops on the owner.** The roles learn what the owner already wants, so the "new" signal decays.
  - **Guard:** track `is_new` acceptance separately. The kill criterion "0 new in 6 meetings" stays.
- **Cost creep.** The Reflector and Curator add about 2 calls per meeting, run only when a signal exists.
  - **Guard:** run them on Haiku, record their cost in `usage.json` under their own role names, and keep the cap from the idea brief.
- **Self-reference.** AgentSI meets about AgentSI.
  - **Guard:** decisions about AgentSI's own gate or eval follow the same approval flow, with the extra flag. A meeting may never write outside `workspace/<product>/`, which `check()` already requires by design.

---

## 7. Claims I could not verify

- **Who&When:** the abstract's 53.5% and 14.2% do not match any single cell of Table 1 (v3); they are probably aggregates. The "spotlight" label appears only in the README.
- **AWM venue:** reported as ICML 2025, but not confirmed on arXiv or in the repo.
- **RoboPhD (arXiv 2604.04347):** a reported finding that a smaller GEPA valset explores more candidates and scores better on test. I could not confirm it from the abstract, so this document does not rely on it.
- **MAGE (arXiv 2607.11944):** a reported "62.4% seed vs 34.0% optimized" comparison. The abstract confirms GEPA at 34.0% and fixed prompts beating reflective optimizers at `N_train=30`, but not the 62.4% figure.
- **SIMBA:** no paper or benchmark numbers exist that I could find.
- **Independent reproductions:** none found for ACE, ExpeL, Voyager, Generative Agents, the Gödel Agent or DGM.
- **Letta:** the default `sleeptime_agent_frequency` value of 5 was seen only in a search snippet.

## 8. Sources

- Reflexion: https://arxiv.org/abs/2303.11366 · https://github.com/noahshinn/reflexion
- Self-Refine: https://arxiv.org/abs/2303.17651 · https://selfrefine.info/
- CRITIC: https://arxiv.org/abs/2305.11738 · https://github.com/microsoft/ProphetNet/tree/master/CRITIC
- Huang et al., LLMs Cannot Self-Correct Reasoning Yet: https://arxiv.org/abs/2310.01798
- Kamoi et al., self-correction survey (TACL 2024): https://arxiv.org/abs/2406.01297
- Olausson et al., self-repair: https://arxiv.org/abs/2306.09896
- ExpeL: https://arxiv.org/abs/2308.10144 · https://github.com/LeapLabTHU/ExpeL
- Agent Workflow Memory: https://arxiv.org/abs/2409.07429 · https://github.com/zorazrw/agent-workflow-memory
- ACE: https://arxiv.org/abs/2510.04618 · https://github.com/ace-agent/ace
- Dynamic Cheatsheet: https://arxiv.org/abs/2504.07952 · https://github.com/suzgunmirac/dynamic-cheatsheet
- MemGPT: https://arxiv.org/abs/2310.08560 · https://github.com/letta-ai/letta
- Letta memory blocks: https://docs.letta.com/guides/agents/memory-blocks
- Sleep-time Compute: https://arxiv.org/abs/2504.13171
- Generative Agents: https://arxiv.org/abs/2304.03442 · https://github.com/joonspk-research/generative_agents
- ReasoningBank: https://arxiv.org/abs/2509.25140 · https://github.com/google-research/reasoning-bank
- Training-Free GRPO: https://arxiv.org/abs/2510.08191
- Memento: https://arxiv.org/abs/2508.16153
- Memp: https://arxiv.org/abs/2508.06433
- Evo-Memory: https://arxiv.org/abs/2511.20857
- OpenAI Cookbook, Self-Evolving Agents: https://cookbook.openai.com/examples/partners/self_evolving_agents/autonomous_agent_retraining
- Voyager: https://arxiv.org/abs/2305.16291 · https://github.com/MineDojo/Voyager
- DSPy: https://arxiv.org/abs/2310.03714 · https://github.com/stanfordnlp/dspy
- MIPROv2: https://arxiv.org/abs/2406.11695 · https://dspy.ai/current/api/optimizers/MIPROv2/
- SIMBA: https://dspy.ai/current/api/optimizers/SIMBA/
- GEPA: https://arxiv.org/abs/2507.19457 · https://dspy.ai/current/api/optimizers/GEPA/overview/
- MAGE: https://arxiv.org/abs/2607.11944
- TextGrad: https://arxiv.org/abs/2406.07496
- OPRO: https://arxiv.org/abs/2309.03409
- APE: https://arxiv.org/abs/2211.01910
- Promptbreeder: https://arxiv.org/abs/2309.16797 · https://proceedings.mlr.press/v235/fernando24a.html
- ADAS: https://arxiv.org/abs/2408.08435
- AFlow: https://arxiv.org/abs/2410.10762
- Darwin Gödel Machine: https://arxiv.org/abs/2505.22954 · https://sakana.ai/dgm/
- Gödel Agent: https://arxiv.org/abs/2410.04444
- Multi-agent debate: https://arxiv.org/abs/2305.14325
- Should we be going MAD?: https://arxiv.org/abs/2311.17371
- Debate or Vote: https://arxiv.org/abs/2508.17536
- Multiagent Finetuning: https://arxiv.org/abs/2501.05707
- MASS: https://arxiv.org/abs/2502.02533
- Who&When: https://arxiv.org/abs/2505.00212
- MAST: https://arxiv.org/abs/2503.13657
- Panickssery et al., self-preference: https://arxiv.org/abs/2404.13076
- Zheng et al., MT-Bench judges: https://arxiv.org/abs/2306.05685
- Anthropic, Building effective agents: https://www.anthropic.com/engineering/building-effective-agents
- Anthropic, Effective context engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Anthropic, Writing effective tools for agents: https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic, Demystifying evals for AI agents: https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
- Anthropic, Agent Skills: https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview · https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills
- Claude Code memory: https://code.claude.com/docs/en/memory
- Claude API memory tool: https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool
