CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_utc TEXT NOT NULL,
    applied_at_tz TEXT NOT NULL
);

CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    short_code TEXT NOT NULL UNIQUE,
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL,
    archived_at_utc TEXT,
    archived_at_tz TEXT
);

CREATE INDEX idx_customers_short_code ON customers(short_code);

CREATE TABLE systems (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    system_type TEXT NOT NULL DEFAULT '',
    hostname TEXT NOT NULL DEFAULT '',
    ip_address TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL,
    archived_at_utc TEXT,
    archived_at_tz TEXT
);

CREATE INDEX idx_systems_customer_id ON systems(customer_id);

CREATE TABLE entries (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    system_id INTEGER REFERENCES systems(id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL CHECK (category IN ('wartung', 'stoerung', 'aenderung', 'installation', 'sonstiges')),
    performed_at_utc TEXT NOT NULL,
    performed_at_tz TEXT NOT NULL,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_entries_customer_id ON entries(customer_id);
CREATE INDEX idx_entries_system_id ON entries(system_id);
CREATE INDEX idx_entries_performed_at_utc ON entries(performed_at_utc);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE entry_tags (
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (entry_id, tag_id)
);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    original_filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL
);

CREATE INDEX idx_attachments_entry_id ON attachments(entry_id);
CREATE INDEX idx_attachments_sha256 ON attachments(sha256);

CREATE TABLE external_refs (
    id INTEGER PRIMARY KEY,
    system_id INTEGER NOT NULL REFERENCES systems(id) ON DELETE CASCADE,
    plugin_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    synced_at_utc TEXT NOT NULL,
    synced_at_tz TEXT NOT NULL
);

CREATE INDEX idx_external_refs_system_id ON external_refs(system_id);

CREATE VIRTUAL TABLE entries_fts USING fts5(
    title,
    body_md,
    content = 'entries',
    content_rowid = 'id'
);

CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, title, body_md) VALUES (new.id, new.title, new.body_md);
END;

CREATE TRIGGER entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, body_md) VALUES ('delete', old.id, old.title, old.body_md);
END;

CREATE TRIGGER entries_au AFTER UPDATE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, body_md) VALUES ('delete', old.id, old.title, old.body_md);
    INSERT INTO entries_fts(rowid, title, body_md) VALUES (new.id, new.title, new.body_md);
END;
