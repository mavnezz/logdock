-- The tags of each issue's events (docs/architecture.md, 4.2): every value
-- with how many events had it, to search issues by tag and to count the
-- users an issue affected (the values of its `user` tag).
CREATE TABLE issue_tag_keys (
    issue_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    -- Events that had the tag.
    events INTEGER NOT NULL,
    -- Values kept in issue_tags; once at the most kept, further ones are
    -- counted in events only.
    distinct_values INTEGER NOT NULL,
    PRIMARY KEY (issue_id, key)
) STRICT, WITHOUT ROWID;

CREATE TABLE issue_tags (
    issue_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    count INTEGER NOT NULL,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (issue_id, key, value)
) STRICT, WITHOUT ROWID;

CREATE INDEX issue_tags_value ON issue_tags (key, value);

-- Events received before now were not counted; they are, once, after the
-- start (logdock_infra::issues::backfill_tags), which deletes this setting
-- when it is done.
INSERT INTO settings (key, value)
VALUES ('issue_tags_backfill', json_object('since', CAST(unixepoch('subsec') * 1000000 AS INTEGER), 'day', -1, 'id', 0));
