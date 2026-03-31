CREATE TABLE IF NOT EXISTS bookmarks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path  TEXT    NOT NULL,
    page       INTEGER NOT NULL,   -- page index (PDF/CBZ) or chapter index (EPUB)
    title      TEXT,               -- optional label (auto-generated or user-provided)
    note       TEXT,               -- user annotation text
    color      TEXT    NOT NULL DEFAULT 'yellow',  -- highlight color name
    created_at TEXT    NOT NULL DEFAULT (datetime('now')),

    UNIQUE(file_path, page, note)  -- prevent exact duplicate bookmarks
);

CREATE INDEX IF NOT EXISTS idx_bookmarks_file_path ON bookmarks(file_path);
