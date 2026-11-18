-- Accounts of the web interface, their sessions, and the audit log
-- (docs/architecture.md, 7). Times are Unix microseconds.
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    locale TEXT NOT NULL DEFAULT 'en',
    password_hash TEXT NOT NULL,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- The token is known to the browser only; its SHA-256 is kept.
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    csrf_token TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    client_ip TEXT,
    user_agent TEXT
) STRICT;

CREATE INDEX sessions_account ON sessions (account_id);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    action TEXT NOT NULL,
    outcome TEXT NOT NULL,
    actor_id INTEGER,
    actor_name TEXT,
    target TEXT,
    reason TEXT,
    client_ip TEXT,
    user_agent TEXT
) STRICT;
