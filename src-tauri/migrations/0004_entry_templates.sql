CREATE TABLE entry_templates (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    system_type TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    body_md TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT 'wartung' CHECK (category IN ('wartung', 'stoerung', 'aenderung', 'installation', 'sonstiges')),
    tags_csv TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_entry_templates_system_type ON entry_templates(system_type);
