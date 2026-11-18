-- The day segments (docs/architecture.md, 5.1): which exist, how many rows
-- they hold and the range of event times in them, so a query opens only
-- the segments that can hold what it asks for.
CREATE TABLE segments (
    kind TEXT NOT NULL,
    day INTEGER NOT NULL,
    rows INTEGER NOT NULL DEFAULT 0,
    min_time INTEGER,
    max_time INTEGER,
    -- 1 once the day is over and the segment was optimized.
    closed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, day)
) STRICT;

-- Log entries per minute (event time), project, stage, service and level:
-- the chart above the list, without reading the segments.
CREATE TABLE log_rollups (
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    service TEXT NOT NULL DEFAULT '',
    level INTEGER NOT NULL,
    minute INTEGER NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (project_id, environment_id, service, level, minute)
) STRICT, WITHOUT ROWID;

CREATE INDEX log_rollups_minute ON log_rollups (minute);

-- Services and hosts seen, for the filters.
CREATE TABLE log_sources (
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    service TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (project_id, environment_id, service, host)
) STRICT;
