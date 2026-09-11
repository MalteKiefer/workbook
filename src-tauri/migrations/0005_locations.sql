CREATE TABLE locations (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    address_line1 TEXT NOT NULL DEFAULT '',
    address_line2 TEXT NOT NULL DEFAULT '',
    postal_code TEXT NOT NULL DEFAULT '',
    city TEXT NOT NULL DEFAULT '',
    country TEXT NOT NULL DEFAULT '',
    phone TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    created_at_utc TEXT NOT NULL,
    created_at_tz TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    updated_at_tz TEXT NOT NULL
);

CREATE INDEX idx_locations_customer_id ON locations(customer_id);
