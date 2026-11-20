-- Tokens an account makes for programs that read the API on its behalf
-- (scripts, Claude through MCP). The token itself is shown once; its
-- SHA-256 is kept, and its first characters to tell tokens apart.
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- read or triage
    scope TEXT NOT NULL,
    prefix TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
) STRICT;

CREATE INDEX api_tokens_account ON api_tokens (account_id);
