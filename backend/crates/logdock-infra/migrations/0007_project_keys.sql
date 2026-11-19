-- A key of the whole project, whose data names its stage, as Sentry's DSNs
-- do: environment_id becomes optional. SQLite cannot change a column's
-- constraint in place, so the table is copied.
CREATE TABLE ingest_keys_new (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    -- NULL: the stage the data names.
    environment_id INTEGER REFERENCES environments (id) ON DELETE CASCADE,
    label TEXT,
    prefix TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
) STRICT;

INSERT INTO ingest_keys_new (id, project_id, environment_id, label, prefix, hash, created_at, last_used_at, revoked_at)
SELECT id, project_id, environment_id, label, prefix, hash, created_at, last_used_at, revoked_at FROM ingest_keys;

DROP TABLE ingest_keys;
ALTER TABLE ingest_keys_new RENAME TO ingest_keys;
CREATE INDEX ingest_keys_project ON ingest_keys (project_id);
