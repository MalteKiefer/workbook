CREATE TABLE tickets (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    system_id INTEGER REFERENCES systems(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    time_spent_minutes INTEGER NOT NULL DEFAULT 0,
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_tickets_customer_id ON tickets(customer_id);
CREATE INDEX idx_tickets_system_id ON tickets(system_id);
