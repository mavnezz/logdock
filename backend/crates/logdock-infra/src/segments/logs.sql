-- A day's log entries. Times are Unix microseconds; attributes a JSON
-- object. The full-text index covers the message and the attributes
-- (docs/architecture.md, 5.3).
CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY,
    time INTEGER NOT NULL,
    received INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    level INTEGER NOT NULL,
    service TEXT,
    host TEXT,
    container TEXT,
    release TEXT,
    message TEXT NOT NULL,
    template TEXT,
    trace_id TEXT,
    span_id TEXT,
    user_id TEXT,
    attributes TEXT NOT NULL,
    redactions INTEGER NOT NULL DEFAULT 0,
    truncated INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX IF NOT EXISTS logs_time ON logs (time);
CREATE INDEX IF NOT EXISTS logs_project_time ON logs (project_id, time);
CREATE INDEX IF NOT EXISTS logs_trace ON logs (trace_id) WHERE trace_id IS NOT NULL;

-- `_` belongs to words, so order_id is one token.
CREATE VIRTUAL TABLE IF NOT EXISTS logs_fts USING fts5 (
    message,
    attributes,
    content = 'logs',
    content_rowid = 'id',
    tokenize = "unicode61 remove_diacritics 2 tokenchars '_'"
);
