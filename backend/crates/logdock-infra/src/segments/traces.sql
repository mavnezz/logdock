-- A day's transactions with their spans (monitoring). Created with the
-- first transaction of the day.
CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY,
    trace_id TEXT NOT NULL,
    span_id TEXT NOT NULL,
    -- request, job, command or other
    kind TEXT NOT NULL,
    -- As counted in the rollups: `GET /orders/{order}`.
    name TEXT NOT NULL,
    status TEXT,
    http_status INTEGER,
    failed INTEGER NOT NULL,
    start INTEGER NOT NULL,
    duration_us INTEGER NOT NULL,
    received INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    release TEXT,
    service TEXT,
    host TEXT,
    span_count INTEGER NOT NULL,
    -- The transaction with its spans as compressed JSON.
    data BLOB NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS transactions_name ON transactions (kind, name, start);
CREATE INDEX IF NOT EXISTS transactions_trace ON transactions (trace_id);
