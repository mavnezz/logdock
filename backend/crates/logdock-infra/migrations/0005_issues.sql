-- Issues: error events grouped (docs/architecture.md, 4.2). The latest
-- event's payload stays with the issue, so its page works after the
-- events' segments are gone.
CREATE TABLE issues (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    environment_id INTEGER NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    culprit TEXT,
    level TEXT NOT NULL,
    -- unresolved, resolved or muted
    status TEXT NOT NULL DEFAULT 'unresolved',
    resolved_at INTEGER,
    resolved_in_next_release TEXT,
    muted_until INTEGER,
    muted_remaining INTEGER,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    first_release TEXT,
    last_release TEXT,
    regressed_at INTEGER,
    last_event_id TEXT,
    last_event TEXT,
    UNIQUE (project_id, environment_id, fingerprint)
) STRICT;

CREATE INDEX issues_last_seen ON issues (last_seen);

-- Events per issue and hour (Unix seconds of the hour's start).
CREATE TABLE issue_hours (
    issue_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    hour INTEGER NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (issue_id, hour)
) STRICT, WITHOUT ROWID;

CREATE TABLE issue_releases (
    issue_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    release TEXT NOT NULL,
    count INTEGER NOT NULL,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (issue_id, release)
) STRICT;
