# anima-agent-rs

Rust port of the [anima-agent](https://github.com/mirainya/anima-agent-clj-main) Clojure framework. This workspace focuses on an observable, schedulable, extensible agent runtime rather than a chat-only shell.

[中文说明](./README.zh-CN.md)

## Overview

`anima-agent-rs` preserves the original framework's runtime-oriented architecture while adapting it to Rust's explicit concurrency model and type system.

The repository currently includes:

- a reusable runtime crate for agent execution, orchestration, permissions, hooks, and tool execution
- a CLI entry point for local interaction and debugging
- an Axum-based web app that serves a React + Vite frontend and streams runtime events over SSE
- shared types, an HTTP SDK, and test utilities for the rest of the workspace

## Workspace crates

The workspace members are defined in `Cargo.toml`:

| Crate | Description |
| --- | --- |
| `anima-types` | Shared types used across runtime, web, SDK, and tests |
| `anima-sdk` | HTTP SDK for interacting with an OpenCode-compatible backend |
| `anima-runtime` | Core runtime: agent execution, classification, orchestration, tools, streaming, permissions, hooks, prompts, metrics |
| `anima-cli` | CLI binary for local interaction and debugging |
| `anima-web` | Axum web server plus local workbench UI and SSE APIs |
| `anima-testkit` | Test helpers and fake executors |

## Architecture

At a high level, the repository is organized as a layered runtime system:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Interfaces                                                           │
│ anima-cli / anima-web / custom channels                              │
├──────────────────────────────────────────────────────────────────────┤
│ Web app                                                              │
│ Axum routes + React/Vite workbench + SSE event stream                │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime bootstrap                                                    │
│ Bus + ChannelRegistry + Dispatcher + Agent + optional CLI channel    │
├──────────────────────────────────────────────────────────────────────┤
│ Core runtime domains                                                 │
│ agent / classifier / orchestrator / execution                        │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime infrastructure                                               │
│ tools / streaming / permissions / hooks / prompt / messages          │
│ bus / channel / context / dispatcher / cache / pipeline / metrics    │
├──────────────────────────────────────────────────────────────────────┤
│ External backend                                                     │
│ OpenCode-compatible model backend accessed through anima-sdk         │
└──────────────────────────────────────────────────────────────────────┘
```

In practice, `anima-runtime` is the system core, `anima-cli` and `anima-web` are entry points, and `anima-sdk` is the boundary used to talk to the upstream model/backend.

## Core execution flow

A typical request travels through the system in this order:

1. An inbound message enters through CLI, web, or another channel.
2. The message is published to the runtime bus.
3. `CoreAgent` prepares session/context state and assembles prompt/input data.
4. The classifier/orchestration layer decides how the request should run.
5. The execution layer may run an agentic loop, invoke tools, evaluate follow-up requirements, and continue iteration.
6. Tool calls go through the tool registry and execution layer, with permission checks and hook integration where required.
7. Results are emitted back onto outbound/internal channels.
8. The dispatcher forwards outward-facing responses to channels, while internal runtime events can be observed by the web workbench over SSE.

This means the project is not just “a chat UI calling a model”, but a runtime that separates message transport, execution control, tool use, permissions, and observability.

## Detailed system architecture

The higher-level layered diagram above is useful for orientation; the following diagram shows the main runtime relationships more explicitly:

```text
User / Operator
   │
   ├─ CLI input
   ├─ Web workbench actions
   └─ Other channel inputs
   │
   ▼
Channel implementations
(CliChannel / WebChannel / custom Channel)
   │
   ▼
Bus (inbound / outbound / internal)
   │
   ├──────────────► internal events ──────────────► SSE forwarder ─────► Web UI
   │
   ▼
CoreAgent
   │
   ├─ session + context preparation
   ├─ prompt assembly
   ├─ classifier
   ├─ orchestrator
   └─ execution driver / turn coordination
   │
   ▼
Agentic loop
   │
   ├─ model request via anima-sdk
   ├─ streaming parser / stream events
   ├─ tool detection and execution
   ├─ requirement evaluation
   └─ iterative continuation until completion or suspension
   │
   ├─ tool call ─► ToolRegistry ─► built-in/custom tools
   │                    │
   │                    ├─ PermissionChecker
   │                    └─ HookRegistry
   │
   ▼
Outbound response + runtime state updates
   │
   ├─ Dispatcher ───────────────────────────────────────────────────────► registered channels
   └─ status / timeline / metrics / jobs ─────────────────────────────► Web APIs
```

Key relationships:

- The **bus** is the runtime message backbone, separating inbound work, outbound responses, and internal events.
- The **agent** owns the stateful execution lifecycle instead of letting channels talk to the model directly.
- The **execution loop** is where model reasoning, tool use, permissions, hooks, and continuation logic meet.
- The **web layer** both submits work and observes runtime internals; it is not only a static dashboard.
- The **dispatcher** is the outward delivery boundary, while SSE focuses on observability rather than final user response transport.

## Design boundaries and non-goals

This repository is best understood as a **runtime-first agent system**, not just a chat frontend.

What it is:

- a runtime that separates transport, execution control, tool use, permissions, hooks, and observability
- a codebase that treats agent execution as a stateful, inspectable process
- a foundation for governed tool-capable workflows rather than only plain text generation

What it is not:

- not just a single-page chat app with a thin model wrapper
- not a fully opaque autonomous agent where all behavior is hidden inside prompts
- not a design where channels call the model directly and bypass runtime control
- not a finished, frozen orchestration platform with all abstractions settled

That boundary is important: many of the project choices only make sense if you treat the runtime itself as the product.

## Module navigation index

If you are new to the repository, these are the fastest entry points by concern:

- **Understand runtime wiring**: `crates/anima-runtime/src/bootstrap.rs`, `crates/anima-runtime/src/lib.rs`
- **Understand agent lifecycle**: `crates/anima-runtime/src/agent/`
- **Understand planning / routing / orchestration**: `crates/anima-runtime/src/classifier/`, `crates/anima-runtime/src/orchestrator/`
- **Understand the agentic loop**: `crates/anima-runtime/src/execution/agentic_loop.rs`, `crates/anima-runtime/src/execution/`
- **Understand tool execution**: `crates/anima-runtime/src/tools/definition.rs`, `crates/anima-runtime/src/tools/registry.rs`, `crates/anima-runtime/src/tools/execution.rs`
- **Understand permissions**: `crates/anima-runtime/src/permissions/`
- **Understand hooks**: `crates/anima-runtime/src/hooks/`
- **Understand message shaping / compaction / normalization**: `crates/anima-runtime/src/messages/`
- **Understand prompt assembly**: `crates/anima-runtime/src/prompt/`
- **Understand streaming model responses**: `crates/anima-runtime/src/streaming/`
- **Understand web integration**: `crates/anima-web/src/main.rs`, `crates/anima-web/src/routes.rs`, `crates/anima-web/src/sse.rs`
- **Understand frontend workbench**: `crates/anima-web/frontend/`

## Runtime module layout

`crates/anima-runtime` has been reorganized into directory-based domains. The public module tree is declared in `crates/anima-runtime/src/lib.rs`.

### Core domains

- `agent/` — core agent engine (CoreAgent facade), task types, executors, worker pool, SuspensionCoordinator, TaskRegistry
- `classifier/` — rule-based, AI-assisted, task, and routing classifiers
- `orchestrator/` — orchestration core, parallel pool, specialist pool
- `execution/` — execution driver, turn coordination, context assembly, requirement judgment
- `runtime/` — event-sourced unified runtime state core (RuntimeDomainEvent → reducer → RuntimeStateSnapshot → projection)
- `tasks/` — domain model definitions (Run / Turn / Task / Suspension / Requirement / ToolInvocation records), lifecycle, scheduling, query
- `transcript/` — message record model (MessageRecord), append, normalizer, pairing

### Infrastructure and support

- `bus/` — inbound/outbound/internal publish-subscribe message bus
- `cache/` — LRU and TTL cache primitives
- `channel/` — channel adapters and session-aware routing
- `context/` — session and context storage/management
- `dispatcher/` — routing, control, balancing, diagnostics, and queueing
- `pipeline/` — processing pipeline building blocks
- `tools/` — tool definitions, registry, execution, and built-in tool implementations
- `streaming/` — streaming API parsing and execution helpers
- `permissions/` — permission policies, types, and checks
- `messages/` — message normalization, lookup, pairing, compaction, and protocol helpers
- `hooks/` — hook registry, runner, and hook types
- `prompt/` — system prompt section assembly
- `bootstrap` / `cli` / `metrics` / `support` — runtime bootstrap, CLI wiring, metrics, and shared utilities

The crate still exposes compatibility re-exports for older module paths, but new documentation should follow the directory-based layout above.

## Runtime responsibilities

The runtime is split by responsibility rather than by a few large source files:

- **Agent layer**: owns the core agent lifecycle, worker pool, task execution entry points, and runtime-facing status.
- **Classifier layer**: decides how incoming work should be interpreted or routed.
- **Orchestrator layer**: coordinates sequential, parallel, or specialist-oriented execution paths.
- **Execution layer**: drives the turn loop, context assembly, requirement evaluation, and model/tool iteration.
- **Tooling and streaming layer**: supports built-in tools, tool execution, streaming parsing, and streaming callbacks.
- **Control layer**: permissions, hooks, prompt assembly, and message normalization keep the runtime behavior structured and policy-aware.
- **Infrastructure layer**: bus, channels, dispatcher, cache, context, pipeline, metrics, and bootstrap support the runtime around the edges.

A useful mental model is:

- `bootstrap.rs` wires the system together
- `bus` carries messages and internal events
- `dispatcher` delivers outbound messages to registered channels
- `agent` + `classifier` + `orchestrator` + `execution` determine what work is done
- `tools` + `permissions` + `hooks` govern how tool-assisted execution is allowed and recorded
- `metrics`, status snapshots, and internal events make the runtime observable

## Web workbench

`anima-web` is an Axum server that hosts the local workbench and runtime APIs.

Current web stack:

- backend: Rust + Axum
- frontend: React 18 + Vite
- realtime transport: SSE (`/api/events`)
- local default URL: <http://localhost:3000>

The frontend build output is expected at `crates/anima-web/src/static/dist`. If the assets are missing, the server returns a fallback page telling you to build the frontend first.

The web app is not a separate backend from the runtime. It bootstraps `anima-runtime`, registers a `WebChannel`, starts forwarding internal bus events, and then exposes:

- the SPA entry page and built frontend assets
- `/api/send` for inbound user messages
- `/api/events` for SSE runtime events
- `/api/status` for runtime/worker/session snapshots
- job-oriented endpoints used by the workbench for review and question-answer flows

That makes `anima-web` both an operator console and a runtime integration surface: it can submit work into the runtime and observe the runtime's internal state in near real time.

Frontend scripts are defined in `crates/anima-web/frontend/package.json`:

| Command | Purpose |
| --- | --- |
| `npm run dev` | Start the Vite dev server |
| `npm run build` | Build production frontend assets |
| `npm run preview` | Preview the built frontend locally |
| `npm run test` | Run frontend tests with Vitest |
| `npm run lint` | Run ESLint for the frontend source |

## Key runtime mechanisms

### Context compaction

The runtime includes automatic context compaction so long-running agent loops do not keep sending the full unbounded history back to the model.

The current mechanism is two-stage:

1. **Microcompact**: old `tool_result` messages before the protected recent-turn boundary are replaced with compact placeholder content.
2. **Snip compact**: if that is not enough, the runtime marks the oldest eligible messages as `filtered` starting from the oldest side.

Important details of the current design:

- token usage is estimated from serialized message size rather than using the model provider's exact tokenizer
- compaction only triggers after a configurable context-window threshold is crossed
- the most recent turns are protected from compaction
- the initial user message is preserved
- filtered messages are excluded when normalizing messages for the upstream API

In other words, the runtime prefers to keep recent reasoning continuity while progressively reducing old tool-heavy history.

### Message normalization

Before calling the upstream model API, internal messages are normalized:

- messages already marked as `filtered` are skipped
- consecutive messages with the same role can be merged
- the API message list is forced to start with a `user` role if necessary

This is the bridge between the runtime's richer internal message state and the simpler API-facing conversation format.

### Prompt assembly

System prompts are assembled from ordered sections instead of one hard-coded string. That allows the runtime to compose prompt text from multiple concerns while keeping the final prompt deterministic.

### Tool execution pipeline

A single tool call follows a structured lifecycle:

1. input schema validation
2. pre-tool hook execution
3. permission check
4. actual tool call
5. post-tool hook execution
6. conversion into `tool_result` messages for the loop

That structure is important because it makes tool use governable rather than ad hoc.

### Permissions and suspension

Permission handling is not just allow-or-deny. The checker can also return an explicit “ask user” decision.

When that happens:

- the current tool call is suspended
- the runtime stores the pending invocation state
- the user is asked for confirmation
- the runtime resumes after an answer arrives

This is one of the core control boundaries in the system.

### Hooks

Hooks currently wrap important runtime moments such as pre/post tool use and pre/post send behavior. They provide extension points for logging, policy enforcement, or side effects without hard-coding all behavior into the execution loop itself.

### Streaming and SSE

There are two different streaming concepts in the repository:

- **model streaming** inside `anima-runtime/streaming`: parses SSE-style upstream responses, accumulates text/tool JSON fragments, and feeds the agentic loop
- **web SSE** inside `anima-web`: forwards runtime internal events to the browser so the workbench can observe progress

One is about consuming the model stream; the other is about exposing runtime state to operators.

## Message model

The runtime uses multiple message representations for different layers of the system:

- **InternalMsg**: the richest runtime form, including `message_id`, optional `tool_use_id`, `filtered` state, and metadata
- **ApiMsg**: the simplified message list sent to the upstream LLM API
- **SdkMsg**: the SDK-facing wrapper format when talking through `anima-sdk`

A useful way to think about them is:

```text
User input / tool result / assistant output
            │
            ▼
InternalMsg (runtime state)
- full content
- tool linkage
- filtered flag
- metadata
            │
            ├─ compact / normalize / pair / transform
            ▼
ApiMsg (upstream request shape)
- simpler role/content representation
            │
            ▼
SdkMsg / provider request layer
```

This split matters because the runtime needs more control metadata than the upstream model API does.

### Message lifecycle in practice

1. user or tool events are captured as internal runtime messages
2. compaction and filtering may modify old internal messages
3. normalization prepares a valid upstream-facing message list
4. upstream responses are parsed back into runtime structures
5. tool outputs re-enter the loop as internal messages with tool linkage

This design keeps the runtime stateful and inspectable without forcing that same complexity onto the external API boundary.

## Control-flow diagram: compaction, tool loop, and permission boundary

```text
Inbound request
   │
   ▼
Assemble internal messages + prompt sections
   │
   ▼
Check context size threshold
   │
   ├─ below threshold ───────────────────────────────────────────────► continue
   │
   └─ above threshold
        │
        ├─ phase 1: compact old tool_result content
        ├─ phase 2: mark oldest eligible messages as filtered
        └─ normalize remaining messages for upstream API
   │
   ▼
Send model request
   │
   ▼
Receive text / tool_use / streaming deltas
   │
   ├─ final text only ───────────────────────────────────────────────► send response
   │
   └─ tool_use detected
        │
        ▼
     validate input
        │
        ▼
     run pre-tool hooks
        │
        ▼
     permission check
        │
        ├─ allow ─► execute tool ─► run post-tool hooks ─► append tool_result ─► continue loop
        ├─ deny  ─► tool error / refusal path
        └─ ask   ─► suspend invocation ─► wait for user answer ─► resume or reject
```

This is the core control loop that makes the runtime both autonomous and governable.

## Typical scenarios

### 1. Plain user question

This is the simplest end-to-end path:

1. The user submits a message from CLI or the web workbench.
2. The message is converted into an inbound bus message.
3. `CoreAgent` restores session/context and prepares the model input.
4. The execution path runs without requiring tools.
5. The final response is emitted to outbound delivery.
6. The user sees the result through CLI output or the web channel.
7. Meanwhile, the runtime can still emit internal events for tracing, metrics, and status views.

### 2. Model triggers `tool_use`

When the model decides external capability is required:

1. The execution layer receives a model response containing text and/or `tool_use` requests.
2. The tool execution layer resolves the requested tool from `ToolRegistry`.
3. The tool input is prepared inside a runtime `ToolContext`.
4. The tool runs and returns a structured `ToolResult`.
5. The tool result is converted back into the message stream.
6. The agentic loop continues with the updated conversation state.
7. The model may either call more tools or produce a final answer.

This loop is what makes the runtime agentic rather than single-shot.

### 3. Tool call blocked for permission confirmation

For permission-sensitive tools, the flow becomes interactive:

1. A tool request is detected during the agentic loop.
2. The runtime builds a permission request and checks it through `PermissionChecker`.
3. If approval is required, the current loop is suspended instead of continuing blindly.
4. The pending tool invocation and related question state are stored by the runtime.
5. The user is asked to approve, reject, or otherwise answer the question.
6. After the user responds, the runtime resumes the suspended path.
7. The tool is either executed or rejected, and the loop continues toward completion.

This keeps risky actions inside an explicit approval boundary instead of burying them inside model output.

### 4. Web workbench observing runtime progress over SSE

The web UI can watch the runtime while a job is executing:

1. The browser subscribes to `/api/events`.
2. `anima-web` listens to the runtime internal bus.
3. Worker state changes and runtime events are forwarded as SSE events.
4. The frontend updates worker cards, timelines, and job views in near real time.
5. The browser can also call `/api/status` to fetch snapshots of workers, sessions, metrics, and jobs.

This gives the project an operator-console feel: the user is not limited to the final answer, but can also inspect how the runtime is progressing.


## Status and observability surfaces

The project exposes several different “views” of runtime state. They are related, but not identical:

- **worker status**: what each worker is doing right now, including busy/idle state and current task previews
- **runtime timeline**: ordered internal events such as message receipt, planning, cache hits, requirement checks, follow-up scheduling, and failures
- **recent execution summaries**: condensed per-message summaries of plan type, status, stage timings, worker assignment, and error codes
- **jobs**: web-facing aggregated views built from accepted work, timeline events, summaries, failures, worker state, reviews, and pending questions
- **metrics**: counters, gauges, histograms, and snapshots for operational visibility
- **recent sessions**: chat/session-oriented slices of recent interaction context

These views answer different questions:

- “What is the runtime doing now?” → worker status / jobs
- “What happened over time?” → timeline
- “How did a specific execution end?” → execution summaries / failures
- “What does the UI need to render?” → jobs + status snapshot APIs
- “How healthy is the system overall?” → metrics

## Terminology

- **job**: the web-facing unit of accepted work, usually derived from an inbound message and enriched with status, events, summaries, worker info, and user-review/question state
- **trace_id**: the end-to-end correlation identifier used to connect runtime events, execution summaries, and follow-up activity across one logical flow
- **message_id**: the concrete message/event identifier associated with a specific inbound request or runtime event record
- **tool_use_id**: the identifier linking a model-issued tool request with the corresponding `tool_result`
- **filtered**: a flag on internal messages meaning the message is intentionally excluded from upstream API normalization, typically due to compaction
- **followup**: an automatically scheduled additional round used when the runtime determines requirements are still unsatisfied after an initial execution
- **runtime timeline**: the ordered stream of internal events that records what happened during execution over time
- **execution summary**: a condensed per-execution result record including status, stage timings, worker assignment, and error information
- **pending question**: a question currently awaiting user input, often created by approval/permission or clarification flows
- **job kind**: whether a job is a main job or a subtask job in orchestration-aware views

## FAQ

### Why does `anima-web` need a frontend build first?
Because the Rust web server serves built assets from `crates/anima-web/src/static/dist`. Without that output, the server can still start, but it intentionally returns a fallback page telling you to build the frontend.

### If `cargo build --workspace` or `cargo test --workspace` can fail, why are they still in the README?
Because those are still the correct top-level workspace commands. The failure is due to current in-progress code in the repository, not because the commands themselves are wrong.

### Why are there both `InternalMsg` and `ApiMsg` instead of one shared message type?
Because the runtime needs richer internal state than the upstream model API understands. Internal messages carry metadata such as filtering and tool linkage, while API messages are normalized down to the external request shape.

### Why keep old re-export paths if the module structure was already reorganized?
Because the codebase is in a transition period. The new directory-based structure is the current source of truth, while compatibility re-exports reduce breakage for older imports during migration.

### Why is context compaction heuristic?
The current compaction logic estimates token usage from serialized message size. That makes it practical and runtime-local, but it is not the same as exact token accounting from the upstream provider.

### What is the difference between runtime events and final user responses?
Runtime events are internal observability signals: planning, worker activity, follow-up scheduling, approval requests, failures, and so on. Final user responses are the outward-facing answers delivered through channels.

## Architecture comparison: anima-agent-rs vs Claude Code vs OpenClaw

This project draws design inspiration from both Claude Code and OpenClaw, but makes distinct architectural choices. The following comparison covers key dimensions.

### Positioning

| Project | Focus | Language |
| --- | --- | --- |
| Claude Code | Developer CLI / IDE coding assistant | TypeScript (Bun) |
| OpenClaw | Multi-channel AI message gateway (Telegram / Discord / Slack / WhatsApp etc.) | TypeScript (Node.js pnpm monorepo) |
| anima-agent-rs | Observable, schedulable agent runtime engine | Rust |

### Agentic loop

**Claude Code**: `src/query.ts` (~1700 lines), an `async function*` generator. Each iteration: compact context → call model → consume streaming response → execute tools → continue. State is rebuilt as an immutable `State` struct each turn. Tool permissions are handled via React state queues + Promise suspension.

**OpenClaw**: three layers — outer `runEmbeddedPiAgent` (session lane queuing + failover retry), middle `runEmbeddedAttempt` (~2000 lines, full attempt lifecycle with streamFn decorator chain), inner tool-use loop delegated to the private `@mariozechner/pi-coding-agent` package.

**anima-agent-rs**: `execution/agentic_loop.rs`, a synchronous `loop {}`: check iteration limit → message pairing repair → optional compaction → normalize → build API payload → call executor → parse response → tool detection and execution → continue. Suspension returns `AgenticLoopSuspension`; resume via `resume_suspended_tool_invocation`.

### State management

| Dimension | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| Primary state | React AppState (Redux-like) | JSON session files + SQLite task registry | Event-sourced RuntimeStateStore |
| Persistence | In-process only | File + SQLite | In-process (snapshot clone) |
| Querying | Direct state read | SQLite query | snapshot() full clone |
| Event sourcing | No | No | Yes (RuntimeDomainEvent → reducer → snapshot) |

### Tool execution

| Dimension | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| Tool definition | Zod schema + React render component | Dynamic via `createOpenClawCodingTools` | `ToolDefinition` trait + `ToolRegistry` |
| Execution mode | Concurrent (`StreamingToolExecutor`) or serial | Delegated to pi-agent | Serial (one at a time) |
| Permissions | Multi-source rule merging (session / settings / hooks) | exec security + sandbox + host | `PermissionChecker` + policy rules |
| Hooks | stop hooks + pre/post tool hooks | Full plugin hook ecosystem (~10 lifecycle hooks) | `HookRegistry` pre/post tool + pre/post send |

### Concurrency model

| Dimension | Claude Code | OpenClaw | anima-agent-rs |
| --- | --- | --- | --- |
| Runtime | Bun single-thread + async/await | Node.js single-thread + Promise lane queues | std::thread + parking_lot::Mutex + Condvar |
| Sub-agents | Recursive `query()` + agentId isolation | ACP protocol spawn/session | WorkerPool + Orchestrator dispatch |
| Cancellation | AbortController | AbortController | AtomicBool flag |

### Strengths of anima-agent-rs

1. **Type safety and compile-time guarantees**: Rust's ownership system catches data races, null pointer issues, and lifetime errors at compile time.
2. **Event-sourced state management**: the only project among the three with event sourcing. All state changes go through `RuntimeDomainEvent`, enabling full state replay and audit.
3. **Structured runtime observability**: timeline, execution summary, failure snapshot, and job views provide multi-dimensional observability that is more systematic than React state or JSON files.
4. **Explicit domain layering**: agent / classifier / orchestrator / execution / tools / permissions / hooks / prompt / messages are each independent with clear responsibility boundaries.
5. **Multi-worker parallel execution**: WorkerPool + Orchestrator supports true multi-task parallelism, while Claude Code and OpenClaw are fundamentally single-threaded.
6. **Three-layer message mapping**: InternalMsg / ApiMsg / SdkMsg decouples runtime internal state from external API formats.

### Weaknesses of anima-agent-rs

1. **CoreAgent is still too large**: ~3200 lines in `core.rs`, handling message loop, session management, event publishing, state upserts, and execution entry points. Ongoing refactoring (SuspensionCoordinator extracted; RequirementCoordinator and RuntimeEventEmitter planned).
2. **Dual state tracking**: `RuntimeStateStore` and `TaskRegistry` track the same data, requiring double writes with manually maintained consistency.
3. **Full snapshot clone on every read**: reading state clones the entire snapshot including all runs, turns, tasks, and transcript. Cost grows linearly with conversation length.
4. **Limited plugin ecosystem**: compared to OpenClaw's npm plugin system and Claude Code's MCP server integration.
5. **Basic context compaction**: Claude Code has snip / microcompact / autocompact with token budget tracking; anima-agent-rs's compaction is simpler.
6. **Sync thread model limitations**: no fine-grained timeout, graceful shutdown, or cancellation; boundary friction with the async web layer (axum/tokio).

### Evolution roadmap

1. Continue CoreAgent decomposition (extract RequirementCoordinator, RuntimeEventEmitter)
2. Unify RuntimeStateStore and TaskRegistry to eliminate double writes
3. Optimize snapshot reads (targeted queries or Arc copy-on-write)
4. Unify event publishing pattern (eliminate 3 duplicate timeline + bus publish sites)
5. Clean up compatibility re-export layer
6. Enrich hooks / plugin mechanisms

## Current limitations

A few important caveats are worth calling out explicitly:

- the workspace documentation is now aligned to the current source layout, but some runtime areas are still under active refactor
- compatibility re-exports still exist in `anima-runtime`, which means the codebase is in a transition period between old and new module paths
- frontend assets must be built before `anima-web` serves the full workbench UI
- the Rust workspace build/test commands are the intended commands, but at the time of this update the repository still contains in-progress code that can cause `cargo build --workspace` and `cargo test --workspace` to fail
- token estimation for context compaction is heuristic, not provider-exact token accounting
- orchestration, question/approval flows, and agent-loop capabilities are present but still evolving

## Developer starting paths

If you want to change a specific area, start here:

- **Change the web UI or workbench UX**
  - start with `crates/anima-web/frontend/`
  - then inspect `crates/anima-web/src/routes.rs` and `crates/anima-web/src/sse.rs`

- **Change runtime HTTP/web integration**
  - start with `crates/anima-web/src/main.rs` and `crates/anima-web/src/routes.rs`
  - then follow the `WebChannel` and status/job APIs

- **Change tool behavior or add built-in tools**
  - start with `crates/anima-runtime/src/tools/definition.rs`
  - then `registry.rs`, `execution.rs`, and `tools/builtins/`

- **Change permission behavior**
  - start with `crates/anima-runtime/src/permissions/types.rs` and `checker.rs`
  - then trace suspended/resumed execution through the agent loop and agent core

- **Change agent loop behavior**
  - start with `crates/anima-runtime/src/execution/agentic_loop.rs`
  - then inspect message normalization, compaction, streaming, and tool execution

- **Change message/context handling**
  - start with `crates/anima-runtime/src/messages/` and `crates/anima-runtime/src/context/`

- **Change orchestration or follow-up logic**
  - start with `crates/anima-runtime/src/classifier/`, `crates/anima-runtime/src/orchestrator/`, and `crates/anima-runtime/src/agent/core.rs`

- **Debug runtime state from the UI side**
  - inspect `/api/status`, `/api/events`, job views, worker status, and runtime timeline handling in `anima-web`

A practical reading order for new contributors is:

1. `README.md`
2. `crates/anima-runtime/src/lib.rs`
3. `crates/anima-runtime/src/bootstrap.rs`
4. `crates/anima-runtime/src/execution/agentic_loop.rs`
5. `crates/anima-web/src/main.rs`
6. `crates/anima-web/src/routes.rs`

## Quick start

### Prerequisites

- Rust toolchain (workspace uses edition 2021)
- Node.js + npm for the web frontend build
- An OpenCode-compatible backend if you want live agent responses

### Build the Rust workspace

```bash
cargo build --workspace
```

### Run the CLI

```bash
cargo run -p anima-cli -- --help
```

### Build the web frontend

```bash
cd crates/anima-web/frontend
npm install
npm run build
```

### Run the web app

```bash
cargo run -p anima-web
```

Then open <http://localhost:3000>.

## Development and test commands

### Rust workspace

```bash
cargo build --workspace
cargo test --workspace
```

### Frontend (`crates/anima-web/frontend`)

```bash
npm install
npm run dev
npm run build
npm run test
npm run lint
```

## Repository structure

```text
anima-agent-rs/
├── Cargo.toml
├── README.md
├── README.zh-CN.md
├── docs/
│   ├── next-phase-plan.md
│   ├── main-agent-orchestration-design.md
│   └── archive/
└── crates/
    ├── anima-cli/
    ├── anima-runtime/
    │   └── src/
    │       ├── agent/
    │       │   ├── core.rs
    │       │   ├── types.rs
    │       │   ├── executor.rs
    │       │   ├── worker.rs
    │       │   ├── registry.rs
    │       │   └── suspension.rs
    │       ├── classifier/
    │       ├── orchestrator/
    │       ├── execution/
    │       ├── runtime/
    │       │   ├── events.rs
    │       │   ├── reducer.rs
    │       │   ├── snapshot.rs
    │       │   ├── projection.rs
    │       │   └── state_store.rs
    │       ├── tasks/
    │       │   ├── model.rs
    │       │   ├── lifecycle.rs
    │       │   ├── scheduler.rs
    │       │   └── query.rs
    │       ├── transcript/
    │       │   ├── model.rs
    │       │   ├── append.rs
    │       │   ├── normalizer.rs
    │       │   └── pairing.rs
    │       ├── tools/
    │       ├── streaming/
    │       ├── permissions/
    │       ├── messages/
    │       ├── hooks/
    │       ├── prompt/
    │       ├── bus/
    │       ├── cache/
    │       ├── channel/
    │       ├── context/
    │       ├── dispatcher/
    │       ├── pipeline/
    │       ├── bootstrap.rs
    │       ├── cli.rs
    │       ├── lib.rs
    │       ├── metrics.rs
    │       └── support.rs
    ├── anima-sdk/
    ├── anima-testkit/
    ├── anima-types/
    └── anima-web/
        ├── frontend/
        └── src/
            ├── main.rs
            ├── routes.rs
            ├── sse.rs
            └── static/
                └── dist/
```

## Further reading

- [Next Phase Plan](./docs/next-phase-plan.md)
- [Main Agent / Subagent Orchestration Design](./docs/main-agent-orchestration-design.md) — historical design background

## Status

The repository already contains a substantial Rust runtime, CLI, and local web workbench. The runtime now includes event-sourced state management (`runtime/`), structured domain models (`tasks/`, `transcript/`), and has begun decomposing CoreAgent into independent coordinators (`SuspensionCoordinator` is complete; `RequirementCoordinator` and `RuntimeEventEmitter` are planned). The project is functional as an agent-runtime codebase, but some subsystems are still actively evolving.

## License

MIT
