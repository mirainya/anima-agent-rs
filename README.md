# anima-agent-rs

Rust port of the [anima-agent](https://github.com/mirainya/anima-agent-clj-main) Clojure framework — a modular agent runtime for building intelligent, multi-agent systems.

## Overview

anima-agent-rs is a faithful rewrite of the original Clojure implementation, preserving the same architecture and behavior while leveraging Rust's performance and type safety. The project is structured as a Cargo workspace with five crates:

| Crate | Description |
|---|---|
| `anima-runtime` | Core runtime — agent lifecycle, message bus, channels, cache, context storage, dispatcher, pipeline, metrics |
| `anima-sdk` | HTTP client SDK — sessions, messages, files, projects |
| `anima-cli` | CLI entry point |
| `anima-types` | Shared type definitions |
| `anima-testkit` | Test utilities and fake executors |

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   anima-cli                      │
├─────────────────────────────────────────────────┤
│                 anima-runtime                    │
│  ┌───────────┐ ┌──────────┐ ┌────────────────┐  │
│  │   Agent   │ │   Bus    │ │   Dispatcher   │  │
│  │  Worker   │ │ Bounded  │ │  Router/LB     │  │
│  │ Specialist│ │ PubSub   │ │ CircuitBreaker │  │
│  │ Parallel  │ └──────────┘ └────────────────┘  │
│  │Orchestrate│                                   │
│  └───────────┘ ┌──────────┐ ┌────────────────┐  │
│                │  Channel  │ │    Pipeline    │  │
│  ┌───────────┐ │  HTTP     │ │ Source→Xform   │  │
│  │   Cache   │ │  RabbitMQ │ │   →Sink        │  │
│  │  LRU/TTL  │ │  CLI      │ └────────────────┘  │
│  │ Eviction  │ │  Session  │                     │
│  └───────────┘ └──────────┘ ┌────────────────┐  │
│                              │    Metrics     │  │
│  ┌───────────┐               │  Prometheus    │  │
│  │  Context  │               └────────────────┘  │
│  │ L1 Memory │                                   │
│  │ L2 File   │                                   │
│  │  Tiered   │                                   │
│  └───────────┘                                   │
├─────────────────────────────────────────────────┤
│           anima-sdk  /  anima-types              │
└─────────────────────────────────────────────────┘
```

## Key Features

- **Agent System** — Worker pool, specialist pool (capability-based routing with LeastLoaded/RoundRobin/Random), parallel execution with fail-fast and min-success-ratio, orchestrator with task decomposition and topological execution
- **Message Bus** — Bounded pub/sub with backpressure, topic-based routing
- **Channel Adapters** — HTTP, RabbitMQ, CLI, session-based channels with streaming support
- **Dispatcher** — Load balancer, circuit breaker, priority queue, diagnostic routing
- **Cache** — LRU with configurable eviction policies, TTL cache with lazy expiration
- **Context Storage** — Tiered storage (L1 in-memory + L2 file-backed), TTL, promotion/demotion between tiers
- **Pipeline** — Source → Transform → Sink processing chains
- **Metrics** — Counters, gauges (including function-based), histograms, summaries, Prometheus export

## Getting Started

### Prerequisites

- Rust 1.70+ (edition 2021)

### Build

```bash
cargo build --workspace
```

### Run

```bash
cargo run -- --help
```

### Test

```bash
cargo test --workspace
```

340 tests covering all modules.

## Project Structure

```
anima-agent-rs/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── anima-cli/          # CLI binary
│   ├── anima-runtime/      # Core runtime
│   │   ├── src/
│   │   │   ├── agent.rs                # Agent lifecycle
│   │   │   ├── agent_worker.rs         # Worker pool
│   │   │   ├── agent_specialist_pool.rs# Specialist routing
│   │   │   ├── agent_parallel_pool.rs  # Parallel execution
│   │   │   ├── agent_orchestrator.rs   # Task orchestration
│   │   │   ├── bus/                    # Message bus
│   │   │   ├── cache/                  # LRU + TTL cache
│   │   │   ├── channel/               # Channel adapters
│   │   │   ├── context/               # Tiered storage
│   │   │   ├── dispatcher/            # Routing + circuit breaker
│   │   │   ├── pipeline/              # Processing pipeline
│   │   │   └── metrics.rs             # Metrics + Prometheus
│   │   └── tests/          # Integration tests
│   ├── anima-sdk/          # HTTP client SDK
│   ├── anima-testkit/      # Test utilities
│   └── anima-types/        # Shared types
```

## License

MIT
