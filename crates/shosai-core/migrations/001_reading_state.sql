CREATE TABLE IF NOT EXISTS reading_state (
    file_path  TEXT PRIMARY KEY NOT NULL,
    page       INTEGER NOT NULL DEFAULT 0,
    zoom       REAL    NOT NULL DEFAULT 1.0,
    updated_at TEXT    NOT NULL DEFAULT (datetime('now'))
);
