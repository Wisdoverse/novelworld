# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-06-07

### Added

#### Core Platform
- 5 Rust microservices: gateway, user-service, novel-service, agent-service, narrative-service
- React/TypeScript frontend with Feature-Sliced Design
- PostgreSQL 18 with pgvector, pg_trgm, uuid-ossp extensions
- Redis 7 for short-term memory caching
- Docker Compose orchestration (7 containers)
- Nginx reverse proxy with SSE support

#### Novel Ingestion
- Chapter splitting with regex patterns (Chinese, English, numbered) + length-based fallback
- LLM-powered character extraction with structured JSON output
- Character relationship graph extraction
- Narrative branch node detection
- Concurrent avatar generation (max 3 parallel) via image generation API
- File upload with PDF text extraction and MIME/size validation

#### Character AI
- 4-layer memory pyramid: short-term (Redis), mid-term (PG summaries), long-term (pgvector semantic search), permanent
- SSE streaming chat with real-time token output
- Anti-spoiler filtering (characters only know events up to reader's current chapter)
- Memory compression (auto-summarize every 20 conversation turns)
- Embedding generation for long-term semantic memory retrieval

#### Narrative Engine
- Branch node detection at key story moments
- Choice submission with LLM-generated consequences
- World state tracking (JSONB: choices, relationships, events)
- Character relationship scoring (0-100 with clamping)

#### Authentication & Security
- JWT authentication with refresh token rotation
- bcrypt password hashing (cost factor 12)
- Rate limiting (configurable, default 500 req/s)
- Ownership authorization on delete operations
- Parameterized SQL queries throughout

#### LLM Client (`crates/llm-client`)
- Unified SDK supporting 20 providers
- 3 native API formats: OpenAI, Anthropic Messages, Gemini generateContent
- Auto-detect from environment variables (`LlmClient::from_env()`)
- CN/International endpoint split for GLM, Doubao, Qwen
- Built-in retry with exponential backoff (3x, 1s/2s/4s)
- Streaming + embeddings support

#### Observability
- Prometheus metrics: request count, latency histogram, in-flight gauge
- X-Request-Id propagation (generate or forward)
- Structured JSON logging with tracing spans on key handlers
- Aggregated health checks (gateway probes all downstream services)
- Graceful shutdown (SIGINT/SIGTERM) on all services

#### Developer Experience
- One-click start script (`./start.sh`) with interactive setup wizard
- Web-based first-run configuration (choose provider, test API key, create account)
- Makefile for common operations
- CI pipeline (GitHub Actions: Rust check/test/clippy + Frontend type-check/build)
- Dependabot with auto-merge for minor/patch updates
- 28 Rust unit tests + 6 frontend tests
- AGENTS.md / CLAUDE.md for AI coding assistant context

### Providers Supported
OpenAI, Anthropic, Gemini, DeepSeek, Doubao, Qwen, GLM, MiniMax, Moonshot, Baichuan, Stepfun, Yi, Spark, Xiaomi, Mistral, Groq, Together, Ollama, vLLM, and any OpenAI-compatible API.
