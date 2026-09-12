ALTER TABLE networks ADD COLUMN location_id INTEGER REFERENCES locations(id) ON DELETE SET NULL;

CREATE INDEX idx_networks_location_id ON networks(location_id);
