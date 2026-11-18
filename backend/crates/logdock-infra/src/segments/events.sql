-- A day's error events (step 5). Created with the first event.
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY,
    event_id TEXT NOT NULL,
    issue_id INTEGER NOT NULL,
    time INTEGER NOT NULL,
    received INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    level INTEGER NOT NULL,
    release TEXT,
    service TEXT,
    host TEXT,
    trace_id TEXT,
    transaction_name TEXT,
    message TEXT NOT NULL,
    data BLOB NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS events_issue ON events (issue_id, time);
CREATE INDEX IF NOT EXISTS events_event_id ON events (event_id);
CREATE INDEX IF NOT EXISTS events_trace ON events (trace_id) WHERE trace_id IS NOT NULL;
