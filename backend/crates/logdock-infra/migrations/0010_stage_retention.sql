-- Stages that keep their data for days of their own get segments of their
-- own (docs/architecture.md, 5.4): a segment is a kind, a day and a
-- retention, `keep` days or 0 for the kind's own. SQLite cannot change a
-- primary key in place, so the catalog is copied.
CREATE TABLE segments_new (
    kind TEXT NOT NULL,
    day INTEGER NOT NULL,
    keep INTEGER NOT NULL DEFAULT 0,
    rows INTEGER NOT NULL DEFAULT 0,
    min_time INTEGER,
    max_time INTEGER,
    -- 1 once the day is over and the segment was optimized.
    closed INTEGER NOT NULL DEFAULT 0,
    -- 1 once what each stage takes of it was measured after it closed.
    measured INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, day, keep)
) STRICT;

INSERT INTO segments_new (kind, day, rows, min_time, max_time, closed)
SELECT kind, day, rows, min_time, max_time, closed FROM segments;

DROP TABLE segments;
ALTER TABLE segments_new RENAME TO segments;

-- What each project's stage takes of a segment: its rows, and its share of
-- the files by the size of its rows (logdock_infra::segments, measure).
CREATE TABLE segment_usage (
    kind TEXT NOT NULL,
    day INTEGER NOT NULL,
    keep INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    environment_id INTEGER NOT NULL,
    rows INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    PRIMARY KEY (kind, day, keep, project_id, environment_id)
) STRICT, WITHOUT ROWID;
