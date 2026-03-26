# anima-agent-rs

Rust port of the [anima-agent](https://github.com/mirainya/anima-agent-clj-main) Clojure framework — a modular agent runtime for building intelligent, observable, multi-agent systems.

[中文说明](./README.zh-CN.md)

## Overview

anima-agent-rs is a Rust workspace focused on agent runtime infrastructure rather than just chat interaction. It preserves the core architecture of the original Clojure implementation while adding Rust's performance, type safety, and explicit concurrency model.

The workspace currently contains six crates:

| Crate | Description |
|---|---|
| `anima-runtime` | Core runtime — agent lifecycle, message bus, worker pool, specialist pool, orchestration, cache, context storage, dispatcher, pipeline, metrics |
| `anima-sdk` | HTTP client SDK for interacting with the OpenCode-compatible backend |
| `anima-cli` | CLI entry point for local interaction and debugging |
| `anima-web` | Axum-based web dashboard with chat UI, SSE event stream, and real-time worker monitoring |
| `anima-types` | Shared type definitions |
| `anima-testkit` | Test utilities and fake executors |

## What makes it different

Compared with typical AI chat apps or lightweight agent demos, this project emphasizes runtime capabilities:

- **Observable execution** — inspect worker status, current task, metrics, and internal events
- **Structured scheduling** — classify requests into direct/single/sequential/parallel/specialist-route plans
- **Composable infrastructure** — bus, channels, worker pools, specialist routing, cache, and context are decoupled components
- **Private deployment friendly** — suitable for local or self-hosted agent systems
- **Rust-first concurrency** — explicit worker lifecycle, bounded channels, condvar-based waiting, and predictable execution paths

## Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│                           Interfaces                                │
│                  anima-cli / anima-web / channels                   │
├─────────────────────────────────────────────────────────────────────┤
│                           anima-runtime                             │
│                                                                     │
│  Inbound Message                                                    │
│       │                                                             │
│       ▼                                                             │
│   Bus / Channel Registry                                            │
│       │                                                             │
│       ▼                                                             │
│   CoreAgent                                                         │
│   ├─ Session / Context management                                   │
│   ├─ Request classification                                          │
│   ├─ Cache lookup                                                    │
│   └─ Orchestration                                                   │
│       │                                                             │
│       ├─ WorkerPool      → general task execution                   │
│       ├─ ParallelPool    → batch / parallel execution               │
│       └─ SpecialistPool  → capability-based routing                 │
│                                                                     │
│   Supporting systems:                                               │
│   - Dispatcher / routing                                            │
│   - LRU + TTL cache                                                 │
│   - Tiered context storage                                          │
│   - Metrics / Prometheus-friendly counters and histograms           │
└─────────────────────────────────────────────────────────────────────┘
```

## Core runtime flow

1. An inbound message is published to the bus.
2. `CoreAgent` ensures session/context state exists.
3. `AgentClassifier` builds an `ExecutionPlan`.
4. `AgentOrchestrator` executes the plan.
5. Tasks are dispatched to `WorkerPool`, `ParallelPool`, or `SpecialistPool`.
6. Results are aggregated and emitted as outbound messages.
7. Web UI can inspect worker status and receive live internal events over SSE.

## Key features

- **Agent runtime**
  - Worker pool with busy/idle tracking and per-worker metrics
  - Capability-based specialist routing with least-loaded / round-robin / random strategies
  - Parallel execution with configurable concurrency, fail-fast mode, and minimum success ratio
  - Orchestration layer with sequential, parallel, and specialist-route execution modes
- **Observability**
  - Real-time worker status API
  - Current task visibility (`task_id`, `task_type`, elapsed time, content preview)
  - SSE event forwarding for web dashboards
- **Message bus**
  - Bounded pub/sub channels with topic separation and backpressure-aware behavior
- **Channel adapters**
  - CLI, HTTP/web, session-aware channels, and extension points for more transports
- **Cache and context**
  - LRU/TTL cache and layered context/session history storage
- **Metrics**
  - Counters, gauges, histograms, and runtime snapshots suitable for diagnostics

## Web dashboard

`anima-web` provides a lightweight local dashboard for interacting with the runtime.

Features:

- Chat-style message input
- Session list in the sidebar
- Real-time SSE event stream
- System status panel
- Worker cards showing:
  - worker status
  - completed/error/timeout metrics
  - current task type
  - current task elapsed time
  - prompt/content preview

Default local URL:

```text
http://localhost:3000
```

## Getting started

### Prerequisites

- Rust 1.70+ (edition 2021)
- An OpenCode-compatible backend available locally if you want live agent responses

### Build the workspace

```bash
cargo build --workspace
```

### Run the CLI

```bash
cargo run -p anima-cli -- --help
```

### Run the web dashboard

```bash
cargo run -p anima-web
```

Then open <http://localhost:3000>.

### Run tests

```bash
cargo test --workspace
```

## Project structure

```text
anima-agent-rs/
├── Cargo.toml
├── README.md
├── README.zh-CN.md
└── crates/
    ├── anima-cli/              # CLI binary
    ├── anima-runtime/          # Core runtime
    │   └── src/
    │       ├── agent.rs                # Agent facade + core loop
    │       ├── agent_worker.rs         # WorkerAgent + WorkerPool
    │       ├── agent_parallel_pool.rs  # Parallel batch execution
    │       ├── agent_specialist_pool.rs# Capability-based specialist routing
    │       ├── agent_orchestrator.rs   # Plan execution and orchestration
    │       ├── agent_classifier.rs     # Request classification
    │       ├── bus/                    # Message bus
    │       ├── cache/                  # LRU + TTL cache
    │       ├── channel/                # Channel adapters
    │       ├── context/                # Tiered storage
    │       ├── dispatcher/             # Routing + circuit breaker
    │       ├── pipeline/               # Processing pipeline
    │       └── support/                # Metrics, helpers, context helpers
    ├── anima-sdk/              # HTTP client SDK
    ├── anima-testkit/          # Test utilities
    ├── anima-types/            # Shared types
    └── anima-web/              # Web UI + Axum server
        └── src/
            ├── main.rs
            ├── routes.rs
            ├── sse.rs
            ├── web_channel.rs
            └── static/index.html
```

## Status

The project already includes a working runtime, CLI, and web dashboard. Some advanced orchestration features are present in the codebase in addition to the currently used execution-plan path, making the runtime a solid base for future multi-agent workflow expansion.

## License

MIT
