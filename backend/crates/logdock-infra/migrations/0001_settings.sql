-- Settings kept in the database rather than the environment, as key and
-- JSON value.
CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
