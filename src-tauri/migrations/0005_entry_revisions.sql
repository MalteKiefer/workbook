CREATE TABLE entry_revisions (
    id INTEGER PRIMARY KEY,
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL,
    category TEXT NOT NULL,
    revised_at_utc TEXT NOT NULL,
    revised_at_tz TEXT NOT NULL
);

CREATE INDEX idx_entry_revisions_entry_id ON entry_revisions(entry_id);
