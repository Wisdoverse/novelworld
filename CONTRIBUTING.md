# Contributing to NovelWorld

Thank you for your interest in contributing! This document provides guidelines for contributing to the project.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/<you>/novelworld.git`
3. Create a branch: `git checkout -b feat/your-feature`
4. Make changes, commit, push, open a PR

## Development Setup

```bash
# Prerequisites: Rust 1.78+, Node.js 22+, Docker
docker compose up -d postgres redis
cargo build --workspace
cd frontend && pnpm install && pnpm dev
```

## Code Standards

### Backend (Rust)

- **Architecture**: DDD + Microservices. See [AGENTS.md](./AGENTS.md) for layer rules.
- **Domain layer** must not import from infrastructure or interface.
- **Port traits** in `domain/ports.rs`, implementations in `infrastructure/`.
- **No shared database** between services. Use HTTP adapters for cross-service queries.
- Use `sqlx::query()` with `.bind()`, not `sqlx::query!()` macro.
- Run `cargo clippy --workspace` before committing.

### Frontend (React/TypeScript)

- **Feature-Sliced Design**: `app` → `pages` → `widgets` → `features` → `entities` → `shared`. Never import upward.
- Zustand for client state, TanStack Query for server state.
- Run `pnpm type-check` and `pnpm lint` before committing.

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add new character relationship visualization
fix: resolve SSE stream disconnection on mobile
docs: update deployment guide for Kubernetes
refactor: extract LLM retry logic into shared crate
test: add integration tests for novel parsing pipeline
chore: upgrade dependencies
```

## Pull Request Process

1. Ensure CI passes (Rust build + test + clippy, Frontend type-check + build)
2. Update documentation if adding new features or changing behavior
3. Add tests for new functionality
4. Keep PRs focused — one feature or fix per PR
5. Fill in the PR template with summary, test plan, and screenshots if UI changes

## Architecture Constraints

These are **blocking** — PRs violating them will not be merged:

- Domain layer has zero dependencies on infrastructure or interface layers
- Services communicate via HTTP, never via shared database tables
- Frontend follows FSD import hierarchy strictly
- All LLM calls go through domain port traits with retry

## Testing

```bash
cargo test --workspace          # Backend (28 tests)
cd frontend && pnpm test        # Frontend (6 tests)
```

## Reporting Bugs

Open an issue with:
- Steps to reproduce
- Expected vs actual behavior
- Environment (OS, Docker version, browser)
- Relevant logs (`docker compose logs <service>`)

## Feature Requests

Open an issue with the `enhancement` label. Describe:
- The problem you're solving
- Your proposed solution
- Alternatives you've considered

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](./LICENSE).
