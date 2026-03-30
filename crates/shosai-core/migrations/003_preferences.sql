CREATE TABLE IF NOT EXISTS preferences (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL, -- stored as text for flexible preference types
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
