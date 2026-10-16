# MailX frontend

React + TypeScript + Vite frontend using shadcn's `base-mira` preset. The current UI provides account navigation, a list, and configuration details. Mail connections, synchronization, reading, composing, and sending are not implemented.

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

Account screens compose shadcn components; install missing components using the existing project preset. Native DOM inside those component implementations is expected. The layout uses a navigation sidebar, account list, and settings detail area, with loading, empty, save, conflict, and failure states.

Business components depend on `MailxClient` in `src/lib/mailx-client.ts`. The desktop adapter invokes the four account commands; Rust DTOs generate `src/generated/accounts.ts`. Keep transient form state separate from those generated wire types. Ordinary browser development has no desktop backend and displays an unavailable-host state; it does not persist accounts to localStorage or report simulated saves.

Passwords are write-only inputs. An edit explicitly preserves or replaces the backend secret, never fills it back into the form, and clears sensitive input after submission. A saved account remains “connection unverified”; saving does not contact a mail server. Application data is owned by the desktop/core service in user-data-dir.

Vitest and Testing Library exercise form behavior and the client boundary using explicit test doubles. Desktop IPC and the real OS credential store require separate host verification. See [account configuration](../docs/architecture/account-configuration.md), [API contracts](../docs/contracts/app-api.md), and [validation](../docs/development/validation.md).
