CREATE TABLE networks (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    cidr TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_networks_customer_id ON networks(customer_id);
