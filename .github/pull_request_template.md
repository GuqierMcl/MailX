## What changed

<!-- Behaviour change, motivation and open limitations. English or Chinese is fine. / 行为变化、动机与未解决的限制。中英文均可。不要包含真实凭证、真实邮件内容或带真实地址的日志。 -->

## Verification

<!-- Which checks ran, see docs/development/validation.md; state honestly what did not run. / 执行了哪些检查，见 docs/development/validation.md；未执行的请如实说明。 -->

- [ ] `git diff --check`
- [ ] Relevant frontend / Rust checks (`bun run check`, `bun run rust:test`, …)
- [ ] Sync, sending, HTML, permission or migration changes include failure-path verification and updated docs / 涉及同步、发送、HTML、权限或迁移时，已补充故障路径验证并更新相关文档
- [ ] User-facing changes added one line under `[Unreleased]` in `CHANGELOG.md`, in English and Chinese on the same line (internal / documentation-only changes can skip this) / 用户可见的改动，已在 `CHANGELOG.md` 的 `[Unreleased]` 下补一行中英双语条目（纯内部 / 文档改动可不填）
- [ ] Bilingual documents updated together: `README.md` / `README.zh-CN.md`, `CONTRIBUTING`, `SECURITY`, `CODE_OF_CONDUCT`, `docs/guides/` — English is the primary version / 双语文档已同步更新，以英文版为准
