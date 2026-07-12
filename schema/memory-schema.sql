CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL CHECK (type IN ('user', 'feedback', 'project', 'reference', 'preference', 'workflow')),
    name TEXT NOT NULL,
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
    last_accessed_at DATETIME,
    origin TEXT NOT NULL DEFAULT 'direct' CHECK (origin IN ('direct', 'import', 'merge', 'bundle', 'sync', 'migration')),
    origin_ref TEXT,
    user_confirmed_at DATETIME,
    UNIQUE(scope, name)
);

CREATE TABLE IF NOT EXISTS ambiguities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uid TEXT NOT NULL UNIQUE CHECK (uid <> ''),
    query TEXT NOT NULL,
    memory_ids TEXT NOT NULL CHECK (json_valid(memory_ids) AND json_type(memory_ids) = 'array'),
    context TEXT,
    resolution TEXT DEFAULT 'pending' CHECK (resolution = 'pending' OR json_valid(resolution)),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    resolved_at DATETIME
);

CREATE TABLE IF NOT EXISTS changelog (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uid TEXT NOT NULL UNIQUE CHECK (uid <> ''),
    memory_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('save', 'update', 'supersede', 'delete', 'merge', 'gc')),
    old_content TEXT,
    new_content TEXT,
    source TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS workflow_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uid TEXT NOT NULL UNIQUE CHECK (uid <> ''),
    memory_id TEXT NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('success', 'failure')),
    note TEXT,
    source TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_name_scope ON memories(name, scope);
CREATE INDEX IF NOT EXISTS idx_type ON memories(type);
CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope);
CREATE INDEX IF NOT EXISTS idx_expires ON memories(expires_at);
CREATE INDEX IF NOT EXISTS idx_source ON memories(source);
CREATE INDEX IF NOT EXISTS idx_valid_until ON memories(valid_until);
CREATE INDEX IF NOT EXISTS idx_access ON memories(access_count);
CREATE INDEX IF NOT EXISTS idx_confidence ON memories(confidence);
CREATE TABLE IF NOT EXISTS graph_semantic_edges (
    id TEXT PRIMARY KEY,
    source_ref TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    relation TEXT NOT NULL,
    confidence TEXT NOT NULL CHECK (confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'rejected', 'superseded')),
    evidence TEXT NOT NULL,
    rationale TEXT,
    source_spans TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_spans) AND json_type(source_spans) = 'array'),
    tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
    generated_by TEXT NOT NULL DEFAULT 'agent' CHECK (generated_by IN ('agent', 'manual', 'import')),
    source TEXT NOT NULL DEFAULT 'agent' CHECK (source IN ('manual', 'agent', 'daily_retro', 'weekly_retro')),
    user_confirmed_at DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    valid_until DATETIME,
    ambiguity_id INTEGER,
    version INTEGER DEFAULT 1 CHECK (version >= 1),
    FOREIGN KEY(ambiguity_id) REFERENCES ambiguities(id)
);

CREATE TABLE IF NOT EXISTS graph_semantic_edge_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uid TEXT NOT NULL UNIQUE CHECK (uid <> ''),
    edge_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    action TEXT NOT NULL CHECK (action IN ('ingest', 'update', 'status', 'merge')),
    snapshot TEXT NOT NULL CHECK (json_valid(snapshot) AND json_type(snapshot) = 'object'),
    source TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS graph_nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    ref_table TEXT,
    ref_id TEXT,
    scope TEXT,
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
    origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS graph_edges (
    id TEXT PRIMARY KEY,
    source_node_id TEXT NOT NULL,
    target_node_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    confidence TEXT NOT NULL CHECK (confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'rejected', 'superseded')),
    evidence TEXT,
    source_ref TEXT,
    scope TEXT,
    weight REAL NOT NULL DEFAULT 1.0,
    origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(source_node_id) REFERENCES graph_nodes(id),
    FOREIGN KEY(target_node_id) REFERENCES graph_nodes(id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_ambiguities_uid ON ambiguities(uid);
CREATE UNIQUE INDEX IF NOT EXISTS idx_changelog_uid ON changelog(uid);
CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_runs_uid ON workflow_runs(uid);
CREATE UNIQUE INDEX IF NOT EXISTS idx_graph_semantic_revisions_uid ON graph_semantic_edge_revisions(uid);

CREATE TRIGGER IF NOT EXISTS enforce_ambiguities_uid_insert
BEFORE INSERT ON ambiguities WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_ambiguities_uid_update
BEFORE UPDATE OF uid ON ambiguities WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_changelog_uid_insert
BEFORE INSERT ON changelog WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_changelog_uid_update
BEFORE UPDATE OF uid ON changelog WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_workflow_runs_uid_insert
BEFORE INSERT ON workflow_runs WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_workflow_runs_uid_update
BEFORE UPDATE OF uid ON workflow_runs WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_graph_semantic_edge_revisions_uid_insert
BEFORE INSERT ON graph_semantic_edge_revisions WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
CREATE TRIGGER IF NOT EXISTS enforce_graph_semantic_edge_revisions_uid_update
BEFORE UPDATE OF uid ON graph_semantic_edge_revisions WHEN NEW.uid IS NULL OR NEW.uid = ''
BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;

CREATE INDEX IF NOT EXISTS idx_changelog_memory ON changelog(memory_id);
CREATE INDEX IF NOT EXISTS idx_changelog_action ON changelog(action);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_memory ON workflow_runs(memory_id);
CREATE INDEX IF NOT EXISTS idx_graph_nodes_kind ON graph_nodes(kind);
CREATE INDEX IF NOT EXISTS idx_graph_nodes_label ON graph_nodes(label);
CREATE INDEX IF NOT EXISTS idx_graph_edges_source ON graph_edges(source_node_id);
CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target_node_id);
CREATE INDEX IF NOT EXISTS idx_graph_edges_relation ON graph_edges(relation);
CREATE INDEX IF NOT EXISTS idx_graph_edges_status ON graph_edges(status);
CREATE INDEX IF NOT EXISTS idx_graph_edges_confidence ON graph_edges(confidence);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_status ON graph_semantic_edges(status);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_source ON graph_semantic_edges(source_ref);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_target ON graph_semantic_edges(target_ref);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_relation ON graph_semantic_edges(relation);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_ambiguity ON graph_semantic_edges(ambiguity_id);
CREATE INDEX IF NOT EXISTS idx_graph_semantic_revisions_edge ON graph_semantic_edge_revisions(edge_id, version);
