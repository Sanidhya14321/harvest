# The Model Is Only Half the Agent

_Engineering reliable AI coding systems with validated edits, local decisions, and evidence-driven execution_

An AI coding agent can understand a bug, propose a reasonable fix, and still spend its next five turns fighting a file edit.

It can explain a failing test correctly, then announce success before the replacement test run finishes. It can have enough context to solve a problem while carrying so much irrelevant output that the useful evidence becomes difficult to find.

These failures deserve an engineering diagnosis. Which information did the agent receive? What did its tools guarantee? Which component decided that an action was safe or a task was complete?

The language model sits inside a larger system. That system decides what the model sees, translates its output into actions, manages concurrent work, and determines how failures propagate.

My argument is that reliable coding agents need three responsibilities designed explicitly: reasoning, judgment, and enforcement. Each benefits from a different kind of machinery.

## 1. Treat editing as a concurrency problem

Consider an agent asked to replace a function. A string-replacement tool might require the model to reproduce the original function exactly before supplying the new version.

That request combines two jobs: designing the change and copying existing bytes. Whitespace, repeated code, or a formatter running between the read and the write can make the patch fail even when the intended change is correct.

Line numbers alone introduce another problem. If the file changes after inspection, line 42 might no longer refer to the statement the agent intended to edit.

This is familiar territory for database and distributed-systems engineers: the writer is acting on an observation that may have become stale.

### Bind the edit to the observed state

A stronger interface returns a version identifier alongside the file content. A subsequent edit includes that identifier and an explicit target range:

```text
read(file)
  -> observed_version, visible_lines

edit(file, observed_version, target_range, replacement)
  -> applied | recoverable_conflict | rejected
```

This is a conceptual contract. The implementation must validate the target against its recorded observation and current file state before applying the change.

It resembles optimistic concurrency control: read a version, propose a mutation, validate the precondition, then commit or report a conflict. A file editor also needs filesystem protections; a version check alone does not solve races during the final write.

If the file has changed, recovery should require evidence that the intended target maps uniquely to the current content. Ambiguous recovery should produce a conflict the agent can resolve with another read.

The model now specifies the new content without reconstructing the old content merely to locate it. The runtime owns target validation.

Interface design has research precedent. SWE-agent studies how purpose-built agent-computer interfaces affect repository navigation, editing, and execution. It supports treating the interface as a variable worth evaluating alongside the model. It does not establish the performance of any particular versioned-edit format. [SWE-agent paper](https://arxiv.org/abs/2405.15793).

## 2. Separate open-ended reasoning from narrow judgment

Writing a migration plan and deciding whether an old test log remains relevant are different workloads.

The first requires open-ended generation and substantial reasoning. The second can often be expressed as a bounded classification or scoring question.

An agent loop repeatedly encounters questions such as:

- Which configured model tier should handle this task?
- How relevant is this earlier tool result to the current request?
- Does a proposed operation warrant additional human review?
- Does the final message suggest that promised work stopped prematurely?

Routing every such question through a general-purpose generative model can add network round trips, token charges, and another response to interpret.

### Introduce a local decision service

A practical architecture keeps the main model responsible for planning and code generation, while a local service answers selected narrow questions:

```text
Task and workspace evidence
          |
          v
Main model proposes an action
          |
          v
Runtime validates arguments and policy
          |
          +--> Local decision service
          |      returns a score or bounded choice
          v
Runtime executes, requests approval, or falls back
          |
          v
Observed result returns to the main model
```

The local service needs a typed response contract: a bounded choice, a numeric score, confidence, and an explicit failure outcome. Runtime validation must reject missing fields, invalid choices, non-finite numbers, and out-of-range scores.

Encoder models are a relevant foundation for classification and retrieval workloads. ModernBERT research evaluates an updated bidirectional encoder on these categories, including code-related retrieval. That makes it relevant background for this architecture; application-specific quality still depends on the checkpoint, evidence, and evaluation. [ModernBERT paper](https://arxiv.org/abs/2412.13663).

Local inference has its own cost: memory, compute, startup, and operational complexity. Its advantage can include avoiding cloud API charges for those decisions.

Under a simplified fixed-latency model, if a local decision costs `L_local`, a cloud decision costs `L_cloud`, and the local service successfully handles fraction `q` of requests, expected latency is:

```text
E[L] = L_local + (1 - q) * L_cloud
```

It beats always using the cloud when `L_local < q * L_cloud`. Real measurements must also include queueing and hardware variability, and any comparison must hold decision quality to an acceptable standard.

## 3. Make failure behavior depend on the consequence

Suppose the local decision service goes offline.

For model routing, the agent can use its configured default model. For context relevance, it can preserve the full context. Both retain the baseline workflow at the cost of an optimization.

Tool approval needs a different response. An unavailable classifier supplies no evidence that a mutation can proceed without additional review.

### Define a fallback contract for each decision

| Decision                         | Conservative fallback                 |
| -------------------------------- | ------------------------------------- |
| Model selection                  | Use the configured default            |
| Context relevance                | Preserve the original context         |
| Supplemental completion judgment | Continue the baseline evaluation path |
| Additional mutation-risk gating  | Require human approval                |

The final row applies when that gating layer is enabled. Existing permission rules remain authoritative throughout.

A classifier contributes a risk signal. The runtime owns authorization, path restrictions, argument validation, and execution. A favorable score must not override a deterministic denial.

### Complete evidence matters more than convenient evidence

Imagine a long shell command whose opening looks harmless but whose final operation deletes data. Classifying a truncated prefix would evaluate a different action from the one about to execute.

A useful invariant is:

> A judgment about part of an action cannot authorize the unseen remainder.

If the complete argument structure exceeds the gate's evidence budget, escalate to review. Preserve nested permission fields during evidence preparation. If extensions modify arguments, perform the relevant checks again against the final action.

For a risk gate, uncertainty should have an explicit operational meaning. It cannot quietly become permission.

## 4. Context compression must preserve a protocol

A coding session accumulates search results, compiler output, intermediate explanations, failed attempts, and superseded plans.

Keeping everything can increase the input sent to the main model. Selecting useful evidence therefore becomes part of the agent's design.

Long-context research also cautions against equating capacity with reliable use. Lost in the Middle found position-dependent retrieval performance in the models and tasks it evaluated. This motivates testing evidence selection, while leaving the behavior of a specific contemporary coding model to measurement. [Lost in the Middle paper](https://arxiv.org/abs/2307.03172).

### Select evidence within explicit constraints

One practical strategy protects user requests, pinned information, and recent interactions, then scores eligible older content against the current task. Candidates are ranked and retained within a budget.

The current task matters. A conversation that moves from debugging authentication to changing a database schema should evaluate older output against the latest request, with the original objective retained where useful.

Scoring itself has limits. An excerpt containing only the beginning and end of a large log can miss important evidence in the middle. Selection quality needs tests that place crucial information in omitted regions.

### Preserve tool-call relationships

An agent conversation is structured data. An assistant message may contain both prose and a tool call; a later tool result references that call.

Flattening the message to text and deleting it can leave the result without its originating call. The resulting history can violate a provider's tool protocol.

Safe pruning therefore needs structural rules. Assistant messages containing tool calls can be excluded from text pruning. Eligible tool-result text can be replaced with a compact omission marker while retaining the surrounding message identity and protocol structure.

Compression must preserve a valid replayable interaction.

### Bind cached decisions to content

Memoizing relevance decisions can avoid repeated scoring and unnecessary changes to a prompt prefix. But a message position is insufficient identity: rewinds and branch switches can place different content at the same index.

A stronger cache key includes:

```text
session identity + chunk identity + content hash
```

The cache also needs bounded capacity and cleanup when its owning session ends. This prevents stale judgments from following reused positions and limits memory growth.

## 5. Give the agent structured evidence

A compiler diagnostic, symbol reference, and debugger stack frame can reduce the amount of inference needed to understand a repository.

For example, a text search for `User` can match a type, a comment, a string, or an unrelated local variable. A semantic rename asks the language server to resolve the symbol and produce workspace edits.

Language Server Protocol defines the interface between development tools and language servers. It provides a foundation for capabilities such as navigation and refactoring. [Official LSP documentation](https://microsoft.github.io/language-server-protocol/).

Runtime bugs need a complementary source of evidence. A debugger can expose the actual stack, scopes, and variable values at a failing point. Debug Adapter Protocol standardizes communication with debugger adapters. [Official DAP documentation](https://microsoft.github.io/debug-adapter-protocol/).

These integrations depend on available servers, adapters, project configuration, and advertised capabilities. Their value is concrete: the agent can inspect program structure and runtime state through tools designed for those observations.

Verification needs the same discipline. A command being submitted is different from it finishing. A background test run should remain unverified until its completed result arrives. A successful exit provides evidence about that command; task completion additionally depends on whether the checks cover the requested behavior.

## 6. A timeout does not necessarily stop computation

Moving decisions into a local service introduces lifecycle problems that are easy to overlook.

Consider a Python service running blocking model inference in a worker thread. Its HTTP deadline expires, so the caller receives a timeout. The inference thread may still be running.

If the service releases the capacity slot immediately, another request can start while the original computation continues. Repeated timeouts can defeat the intended concurrency limit and exhaust compute or memory.

### Account for the worker's actual lifetime

A bounded scheduler should limit active inference and queued requests. It should include queueing in the deadline, prevent expired queued work from starting, and retain a running job's capacity slot until that computation actually ends.

Caller lifetime and computation lifetime are separate states.

This distinction affects fail-open behavior too. The agent may have resumed its baseline workflow while the sidecar is still consuming resources. A usable fallback must coexist with accurate accounting for unfinished work.

## 7. Applying these ideas in Harvest

Harvest brings these mechanisms together in a terminal coding-agent implementation built on Pi by Mario Zechner. Its implementation provides a useful case study of how model interaction, local judgments, and deterministic runtime checks fit together.

Its default Hashline editing contract identifies an observed file snapshot with a tag and expresses changes against original line anchors. A simplified example in its actual patch syntax is:

```text
[src/example.ts#1A2B]
PUT 4.=4:
+const value = 2;
```

Here, `1A2B` represents the recorded snapshot tag, and the operation replaces original line 4. The runtime checks snapshot history and visible ranges; stale edits attempt recovery only when that history proves a unique safe result. The short tag participates in a session snapshot protocol and should not be interpreted as a cryptographic integrity guarantee.

> **Laya removal note (2026-10-06):** the Laya local decision sidecar described
> below was removed from Harvest runtime, packaging, setup, and active docs.
> What follows is historical design context, kept for the fallback-contract
> reasoning — not current behavior. Approvals now follow configured
> permissions (`tools.approvalMode`).

Harvest historically integrated Laya through a local Python/FastAPI sidecar using the upstream `convaiinnovations/laya-typed-decisions` checkpoint. Laya provided typed decision outputs for narrow questions. [Laya checkpoint and model card](https://huggingface.co/convaiinnovations/laya-typed-decisions).

The historical integration applied different fallback contracts at different decision points. With Laya gating enabled, write and execution tiers received additional risk checks, while read-tier calls bypassed that check. Invalid answers, incomplete evidence, and service failures required approval (following configured permissions). Context-pruning failures retained the original context.

The historical pruning implementation protected assistant tool-call messages, scored eligible older content in bounded batches, and keyed cached decisions to session and content identity. The sidecar scheduler preserved inference capacity until timed-out prediction work actually finished. LSP and DAP integrations provide semantic and runtime observations.

These mechanisms make the design inspectable. Claims about lower latency, lower token use, or better task success still require controlled measurements. The architecture supplies specific hypotheses and failure modes to evaluate.

## 8. Measure the system that does the work

A useful evaluation keeps the main model and task set fixed, then varies one mechanism at a time.

| Mechanism             | Measurement                                        | Regression to challenge                         |
| --------------------- | -------------------------------------------------- | ----------------------------------------------- |
| Snapshot-bound edits  | Correct application rate and retries               | Concurrent changes and ambiguous targets        |
| Local decisions       | End-to-end p50/p95 latency and decision errors     | Slow hardware, bad answers, and service outages |
| Context pruning       | Input tokens and final task success                | Missing evidence and invalid tool histories     |
| Risk gating           | Missed dangerous actions and unnecessary approvals | Destructive command tails and revised arguments |
| Inference scheduling  | Active work and queue depth during timeouts        | Capacity released before computation ends       |
| Verification tracking | Completion claims backed by finished checks        | Background work reported as already verified    |

Token savings are useful when they preserve task quality. Low latency is useful when it preserves decision quality. A high edit-application rate is useful when the intended change lands in the intended location.

The opportunity is to build environments in which a model can act on clear evidence, express changes precisely, and recover from failures without losing the system's guarantees.

A capable coding agent needs that engineering around it: interfaces that validate intent, judgments with bounded responsibilities, and a runtime that remains accountable for what actually happened.
