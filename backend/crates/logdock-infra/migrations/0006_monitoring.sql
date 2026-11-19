-- Monitoring (docs/architecture.md, 4.3).

-- Requests, jobs, commands, queries, outgoing requests and cache operations
-- per minute (Unix seconds of its start), project, stage and name, with a
-- histogram of durations: b0 up to 1 ms … b14 above 30 s (the bounds are in
-- logdock-performance, domain/rollup.rs). `flagged` counts the failed ones,
-- for the cache the misses.
CREATE TABLE perf_rollups (
    kind TEXT NOT NULL,
    minute INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    count INTEGER NOT NULL,
    flagged INTEGER NOT NULL,
    total_us INTEGER NOT NULL,
    max_us INTEGER NOT NULL,
    b0 INTEGER NOT NULL DEFAULT 0, b1 INTEGER NOT NULL DEFAULT 0, b2 INTEGER NOT NULL DEFAULT 0,
    b3 INTEGER NOT NULL DEFAULT 0, b4 INTEGER NOT NULL DEFAULT 0, b5 INTEGER NOT NULL DEFAULT 0,
    b6 INTEGER NOT NULL DEFAULT 0, b7 INTEGER NOT NULL DEFAULT 0, b8 INTEGER NOT NULL DEFAULT 0,
    b9 INTEGER NOT NULL DEFAULT 0, b10 INTEGER NOT NULL DEFAULT 0, b11 INTEGER NOT NULL DEFAULT 0,
    b12 INTEGER NOT NULL DEFAULT 0, b13 INTEGER NOT NULL DEFAULT 0, b14 INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, minute, project_id, environment_id, name)
) STRICT, WITHOUT ROWID;

CREATE INDEX perf_rollups_name ON perf_rollups (kind, name, minute);
CREATE INDEX perf_rollups_minute ON perf_rollups (minute);

-- Scheduled tasks, created with their first check-in.
CREATE TABLE monitors (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    environment_id INTEGER NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    -- crontab or interval; NULL without a schedule.
    schedule_type TEXT,
    schedule TEXT,
    interval_unit TEXT,
    timezone TEXT,
    margin_s INTEGER,
    max_runtime_s INTEGER,
    -- new, ok, failing or missed
    health TEXT NOT NULL DEFAULT 'new',
    created_at INTEGER NOT NULL,
    last_check_in INTEGER,
    last_status TEXT,
    expected_after INTEGER NOT NULL,
    UNIQUE (project_id, environment_id, slug)
) STRICT;

CREATE TABLE check_ins (
    id INTEGER PRIMARY KEY,
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    check_in_id TEXT NOT NULL,
    -- in_progress, ok, error, missed or timeout
    status TEXT NOT NULL,
    started INTEGER NOT NULL,
    finished INTEGER,
    duration_us INTEGER,
    release TEXT,
    trace_id TEXT,
    UNIQUE (monitor_id, check_in_id)
) STRICT;

CREATE INDEX check_ins_started ON check_ins (monitor_id, started);
CREATE INDEX check_ins_running ON check_ins (started) WHERE status = 'in_progress';
