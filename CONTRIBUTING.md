# Contributing to MailX

English | [简体中文](CONTRIBUTING.zh-CN.md)

MailX has a Rust mail core, a React + TypeScript + Vite + shadcn frontend, a Tauri desktop host and a local MCP server. Receiving, reading, search, organizing and draft/attachment management are implemented. Start with the [documentation index](docs/README.md), the [architecture overview](docs/architecture/overview.md) and the [roadmap](docs/roadmap/README.md), which separate what exists today, hand-tested evidence and future plans.

## How we work

1. Pick one independently verifiable task within a milestone and state the expected behaviour and the evidence that will show it works.
2. Send the implementation together with the matching architecture, contract and test notes. For interface changes, describe the callers and compatibility impact first.
3. Documentation separates initial sketches, later designs and implemented behaviour. A starting sketch can be replaced by a new design; later work is not required to follow it mechanically. Mark something implemented only once the code and the verification evidence both exist.
4. Before committing, run `git diff --check` and review relative links for sensitive information. Rust changes need formatting, Clippy, tests and the relevant build; see the [validation strategy](docs/development/validation.md).

## Project conventions

- Rust uses Cargo and the toolchain pinned in the repository, and `Cargo.lock` is committed. The frontend uses a Bun workspace and the root `bun.lock`. The browser mock is for UI development; real mail and system features must be verified through the desktop host.
- The Rust core merges business and shared implementations and has no separate infra crate; it does not depend on Tauri or Axum. The desktop UI and MCP share the core application services, and the HTTP host will connect later.
- Rust uses `snake_case` internally and JSON crossing a boundary uses `camelCase`; structured errors and permissions are handled once in the core service.
- Never commit credentials, message bodies, real attachments, raw model prompts or local connection configuration, and keep them out of logs and test snapshots. Test mail uses synthetic data.
- Changes to sync, sending, HTML, permissions or migrations need failure-path verification; ordinary documentation edits do not need new tests.
- `README.md` and `README.zh-CN.md` are updated together and keep the same section structure. `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md` and the user guide follow the same rule, with English as the primary language.
- Commit messages follow Conventional Commits, for example `docs(architecture): define mailbox sync lifecycle`.
- User-facing changes add one line to `[Unreleased]` in `CHANGELOG.md` in the PR, written in English and Chinese; bump versions only through `bun run version:bump` / `version:set`. Release steps are in the [release process](docs/development/release.md).

A PR description explains the behaviour change, the verification results and any open limitations. To adjust an earlier plan, update the matching documentation; major choices can record their trade-offs in an [ADR](docs/adr/README.md), and changing a starting sketch does not need an extra approval step.

## Open source and security

The repository is [GuqierMcl/MailX](https://github.com/GuqierMcl/MailX), licensed under the [Apache License 2.0](LICENSE). Contributions are licensed under the same terms. By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

- Report security issues privately as described in [SECURITY.md](SECURITY.md); do not open a public issue.
- Never paste real credentials, real mail content or full logs containing real addresses into issues, pull requests, logs or screenshots. Reproduce with synthetic data.
