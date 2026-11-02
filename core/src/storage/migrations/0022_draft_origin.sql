-- Who created a draft: 'user' (written in the app, including replies and
-- forwards) or 'mcp' (created through the local MCP server). Set by the host at
-- creation and never changed by updates; existing rows are the user's own.
ALTER TABLE drafts ADD COLUMN origin TEXT NOT NULL DEFAULT 'user' CHECK (origin IN ('user', 'mcp'));
