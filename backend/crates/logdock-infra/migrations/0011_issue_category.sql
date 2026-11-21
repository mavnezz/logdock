-- What kind of problem an issue is (docs/architecture.md, 4.2): an error,
-- or a performance problem Logdock found in a trace, whose events carry the
-- tag `issue.type: performance`. Lists show the errors unless asked.
ALTER TABLE issues ADD COLUMN category TEXT NOT NULL DEFAULT 'error';

UPDATE issues SET category = 'performance'
WHERE CASE WHEN json_valid(last_event) THEN json_extract(last_event, '$.tags."issue.type"') END = 'performance';

CREATE INDEX issues_category ON issues (category, last_seen);
