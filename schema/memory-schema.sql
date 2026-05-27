CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL CHECK (type IN ('user', 'feedback', 'project', 'reference', 'preference', 'workflow')),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    content TEXT,
    tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
    scope TEXT DEFAULT 'global',
    source TEXT NOT NULL CHECK (source IN ('manual', 'agent', 'daily_retro', 'weekly_retro')),
    confidence TEXT DEFAULT 'medium' CHECK (confidence IN ('high', 'medium', 'low')),
    protected BOOLEAN DEFAULT FALSE CHECK (protected IN (0, 1)),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME,
    valid_until DATETIME,
    superseded_by TEXT,
    version INTEGER DEFAULT 1 CHECK (version >= 1),
    access_count INTEGER DEFAULT 0 CHECK (access_count >= 0),
    last_accessed_at DATETIME
);

CREATE TABLE IF NOT EXISTS ambiguities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query TEXT NOT NULL,
    memory_ids TEXT NOT NULL CHECK (json_valid(memory_ids) AND json_type(memory_ids) = 'array'),
    context TEXT,
    resolution TEXT DEFAULT 'pending' CHECK (resolution = 'pending' OR json_valid(resolution)),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    resolved_at DATETIME
);

CREATE TABLE IF NOT EXISTS changelog (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('save', 'update', 'supersede', 'delete', 'merge', 'gc')),
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
