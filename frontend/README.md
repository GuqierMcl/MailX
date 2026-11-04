# MailX frontend

React + TypeScript + Vite frontend using shadcn's `base-mira` preset. The desktop UI provides account management and connection tests, a unified inbox and folder views, local full-text search, isolated reading, mail actions, rich-text composing in separate windows, attachments and inline images, and the send queue. Settings include Chinese / English text, notifications, background behavior, login startup, local MCP and application updates.

Run commands from the repository root so `package.json` and `bun.lock` remain the workspace entry point:

```sh
bun install --frozen-lockfile
bun run dev
bun run tauri dev
bun run typecheck
bun run lint
bun run build
bun run frontend:test
bun run types:generate
bun run types:check
```

Account screens compose shadcn components; install missing components using the existing project preset. The application shell uses the project's own components in `src/features/shell/`. Follow the [UI design language](../docs/architecture/ui-design.md): navigation and the title bar share a canvas, and the virtualized mail list and reader form one island without internal divider lines or card borders.

Business components depend on `MailxClient` in `src/lib/mailx-client.ts`. The desktop adapter invokes Tauri commands; Rust DTOs generate `src/generated/`. Keep transient form state separate from those generated wire types. Ordinary browser development uses the explicit mock client for a UI preview; it does not connect to mail servers or prove desktop integration. Application mail data and credentials are owned by the desktop/core service.

Passwords are write-only inputs. An edit explicitly preserves or replaces the backend secret, never fills it back into the form, and clears sensitive input after submission. Saving configuration and verifying a connection are separate operations; after a successful save, the desktop host starts or restarts background synchronization. Email HTML is untrusted and rendered through the isolated body protocol in an empty-sandbox iframe without Tauri IPC access.

Vitest and Testing Library exercise account forms, the workbench, composing, settings, updates, MCP controls and the client boundary using explicit test doubles. Desktop IPC, real mail servers and the OS credential store require separate verification. Built-in AI and the Web/server host are planned. See [account configuration](../docs/architecture/account-configuration.md), [API contracts](../docs/contracts/app-api.md), and [validation](../docs/development/validation.md).
