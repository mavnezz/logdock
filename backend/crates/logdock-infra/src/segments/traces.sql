-- A day's transactions with their spans (monitoring, R2).
CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY,
    trace_id TEXT NOT NULL,
    span_id TEXT NOT NULL,
    parent_span_id TEXT,
    name TEXT NOT NULL,
    op TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT,
    http_status INTEGER,
    start INTEGER NOT NULL,
    duration_us INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    release TEXT,
    service TEXT,
    host TEXT,
    span_count INTEGER NOT NULL,
    data BLOB NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS transactions_name ON transactions (project_id, kind, name, start);
CREATE INDEX IF NOT EXISTS transactions_trace ON transactions (trace_id);
