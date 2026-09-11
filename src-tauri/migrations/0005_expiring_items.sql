CREATE TABLE expiring_items (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    system_id INTEGER REFERENCES systems(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('ssl_certificate', 'domain', 'license', 'contract', 'warranty', 'sonstiges')),
    label TEXT NOT NULL,
    expires_on TEXT NOT NULL,
    reminder_days_before INTEGER NOT NULL DEFAULT 30,
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_expiring_items_customer_id ON expiring_items(customer_id);
CREATE INDEX idx_expiring_items_system_id ON expiring_items(system_id);
CREATE INDEX idx_expiring_items_expires_on ON expiring_items(expires_on);
