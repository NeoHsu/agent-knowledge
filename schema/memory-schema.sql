CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    content TEXT,
    tags TEXT,
    scope TEXT DEFAULT 'global',
    source TEXT NOT NULL,
    confidence TEXT DEFAULT 'medium',
    protected BOOLEAN DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME,
    valid_until DATETIME,
    superseded_by TEXT,
    version INTEGER DEFAULT 1,
    access_count INTEGER DEFAULT 0,
    last_accessed_at DATETIME
);

CREATE TABLE IF NOT EXISTS ambiguities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,
    memory_ids TEXT NOT NULL,
    context TEXT,
    resolution TEXT DEFAULT 'pending',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    resolved_at DATETIME
);

CREATE TABLE IF NOT EXISTS changelog (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL,
    action TEXT NOT NULL,
    old_content TEXT,
    new_content TEXT,
    source TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_type ON memories(type);
CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope);
CREATE INDEX IF NOT EXISTS idx_expires ON memories(expires_at);
CREATE INDEX IF NOT EXISTS idx_source ON memories(source);
CREATE INDEX IF NOT EXISTS idx_valid_until ON memories(valid_until);
CREATE INDEX IF NOT EXISTS idx_access ON memories(access_count);
CREATE INDEX IF NOT EXISTS idx_confidence ON memories(confidence);
CREATE INDEX IF NOT EXISTS idx_changelog_memory ON changelog(memory_id);
CREATE INDEX IF NOT EXISTS idx_changelog_action ON changelog(action);
