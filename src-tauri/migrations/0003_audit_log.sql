CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    at_utc TEXT NOT NULL,
    at_tz TEXT NOT NULL
);

CREATE INDEX idx_audit_log_entity ON audit_log(entity_type, entity_id);
