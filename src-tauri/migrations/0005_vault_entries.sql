CREATE TABLE vault_entries (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    system_id INTEGER REFERENCES systems(id) ON DELETE RESTRICT,
    label TEXT NOT NULL,
    username TEXT NOT NULL DEFAULT '',
    secret_encrypted TEXT,
    url TEXT NOT NULL DEFAULT '',
    notes_encrypted TEXT,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_vault_entries_customer_id ON vault_entries(customer_id);
CREATE INDEX idx_vault_entries_system_id ON vault_entries(system_id);
