CREATE TABLE IF NOT EXISTS books (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT    NOT NULL,
    author     TEXT,
    format     TEXT    NOT NULL,  -- 'pdf', 'epub', 'cbz'
    file_path  TEXT    NOT NULL UNIQUE,
    cover_blob BLOB,
    progress   REAL    NOT NULL DEFAULT 0.0,  -- 0.0 to 1.0
    date_added TEXT    NOT NULL DEFAULT (datetime('now')),
    last_read  TEXT
);
