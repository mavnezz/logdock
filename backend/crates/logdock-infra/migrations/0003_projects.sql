-- Projects (applications), their environments (stages), ingest keys and
-- the counts of what ingest accepted and refused.
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- ProjectSettings as JSON (notification addresses, redaction).
    settings TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE environments (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (project_id, name)
) STRICT;

-- The key is the DSN's public key; its SHA-256 is kept, and its first
-- characters to tell keys apart.
CREATE TABLE ingest_keys (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    environment_id INTEGER NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    label TEXT,
    prefix TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
) STRICT;

CREATE INDEX ingest_keys_project ON ingest_keys (project_id);

-- Per project, key (0: none was valid) and UTC day (days since the epoch).
CREATE TABLE ingest_counts (
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    key_id INTEGER NOT NULL DEFAULT 0,
    day INTEGER NOT NULL,
    accepted INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    -- {"reason": count}
    rejected TEXT NOT NULL DEFAULT '{}',
    last_reason TEXT,
    last_detail TEXT,
    last_at INTEGER,
    PRIMARY KEY (project_id, key_id, day)
) STRICT;
