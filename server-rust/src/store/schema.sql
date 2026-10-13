
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);

CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  persona TEXT NOT NULL DEFAULT '',
  route_id TEXT NOT NULL,
  preset_id TEXT NOT NULL,
  effort TEXT,
  temperature REAL,
  avatar TEXT,
  pinned INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  settings_json TEXT NOT NULL DEFAULT '{}',
  revision INTEGER NOT NULL DEFAULT 1,
  character_rev INTEGER NOT NULL DEFAULT 0,
  head_interaction_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  deleted_at TEXT,
  deleted_batch TEXT
);

CREATE TABLE IF NOT EXISTS character_revisions (
  conv_id TEXT NOT NULL,
  rev INTEGER NOT NULL,
  text TEXT NOT NULL,
  hash TEXT NOT NULL,
  source TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (conv_id, rev)
);

-- Interaction = user message + associated replies (parts). The tree (parent_id)
-- keeps the branches; the active path is the chain starting at conversations.head_interaction_id.
CREATE TABLE IF NOT EXISTS interactions (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  parent_id TEXT,
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL DEFAULT 'turn',
  user_text TEXT,
  user_ts TEXT,
  user_edited INTEGER NOT NULL DEFAULT 0,
  edit_of TEXT,
  created_at TEXT NOT NULL,
  deleted_at TEXT,
  deleted_batch TEXT
);
CREATE INDEX IF NOT EXISTS ix_inter_conv ON interactions(conv_id, parent_id);

CREATE TABLE IF NOT EXISTS parts (
  id TEXT PRIMARY KEY,
  interaction_id TEXT NOT NULL,
  idx INTEGER NOT NULL,
  selected_version_id TEXT,
  created_at TEXT NOT NULL,
  deleted_at TEXT,
  deleted_batch TEXT,
  UNIQUE (interaction_id, idx)
);

CREATE TABLE IF NOT EXISTS versions (
  id TEXT PRIMARY KEY,
  part_id TEXT NOT NULL,
  content TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,            -- generating | complete | length | interrupted | error
  origin TEXT NOT NULL,            -- generate | continue | regenerate | edit | legacy
  generation_id TEXT,
  route_id TEXT,
  preset_id TEXT,
  effort TEXT,
  usage_json TEXT,
  created_at TEXT NOT NULL,
  finished_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_versions_part ON versions(part_id);

CREATE TABLE IF NOT EXISTS generations (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  op_id TEXT,
  interaction_id TEXT,
  part_id TEXT,
  version_id TEXT,
  mode TEXT NOT NULL,
  status TEXT NOT NULL,            -- running | complete | length | interrupted | empty | error
  route_id TEXT,
  preset_id TEXT,
  effort TEXT,
  request_json TEXT,
  context_json TEXT,
  usage_json TEXT,
  error TEXT,
  started_at TEXT NOT NULL,
  ended_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_gen_op ON generations(conv_id, op_id) WHERE op_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS operations (
  conv_id TEXT NOT NULL,
  op_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  result_json TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY (conv_id, op_id)
);

-- Elenco (personagens) -----------------------------------------------------------------
CREATE TABLE IF NOT EXISTS characters (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  name TEXT NOT NULL,
  name_key TEXT NOT NULL,
  aliases_json TEXT NOT NULL DEFAULT '[]',
  description TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL DEFAULT 'npc',  -- main | npc | user
  avatar TEXT,
  source TEXT NOT NULL DEFAULT 'manual', -- card | narrative | manual
  confirmed INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1,
  first_interaction_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (conv_id, name_key)
);

-- Memory --------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS memories (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  subject TEXT,
  content TEXT NOT NULL,
  source_interaction_id TEXT,
  source_version_id TEXT,
  importance REAL NOT NULL DEFAULT 0.5,
  confidence REAL NOT NULL DEFAULT 0.8,
  visibility TEXT NOT NULL DEFAULT 'public',   -- public | private (restrito por memory_knowledge)
  status TEXT NOT NULL DEFAULT 'current',      -- current | historical | rejected
  validity TEXT NOT NULL DEFAULT 'ok',         -- ok | inactive (ramo/trecho invalidado)
  review_state TEXT NOT NULL DEFAULT 'ok',     -- ok | pending_review
  pinned INTEGER NOT NULL DEFAULT 0,           -- pinned by the user: always included in the context
  superseded_by TEXT,
  conv_revision INTEGER,
  created_at TEXT NOT NULL,
  last_confirmed_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_mem_conv ON memories(conv_id, status, validity);

CREATE TABLE IF NOT EXISTS memory_knowledge (
  memory_id TEXT NOT NULL,
  entity_key TEXT NOT NULL,
  entity_name TEXT NOT NULL,
  state TEXT NOT NULL,            -- knows | believes | unaware
  via TEXT,                       -- witnessed | heard | told | discovered | received | inferred
  belief TEXT,
  evidence TEXT,
  source_interaction_id TEXT,
  PRIMARY KEY (memory_id, entity_key)
);

CREATE TABLE IF NOT EXISTS scene_states (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  interaction_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  location TEXT,
  time_text TEXT,
  mood TEXT,
  present_json TEXT NOT NULL DEFAULT '[]',
  objects_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_scene_conv ON scene_states(conv_id, seq);

CREATE TABLE IF NOT EXISTS character_states (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  interaction_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  entity_key TEXT NOT NULL,
  entity_name TEXT NOT NULL,
  location TEXT,
  position TEXT,
  holding_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_cstate_conv ON character_states(conv_id, entity_key, seq);

CREATE TABLE IF NOT EXISTS relationships (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  a_key TEXT NOT NULL,
  a_name TEXT NOT NULL,
  b_key TEXT NOT NULL,
  b_name TEXT NOT NULL,
  description TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'current',
  validity TEXT NOT NULL DEFAULT 'ok',
  superseded_by TEXT,
  source_interaction_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS threads (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  text TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'open',
  validity TEXT NOT NULL DEFAULT 'ok',
  opened_interaction_id TEXT,
  resolved_interaction_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS interaction_notes (
  interaction_id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  summary TEXT NOT NULL,
  version_ids_json TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS episodes (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER NOT NULL,
  interaction_ids_json TEXT NOT NULL,
  summary TEXT NOT NULL,
  validity TEXT NOT NULL DEFAULT 'ok',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS embeddings (
  owner_type TEXT NOT NULL,
  owner_id TEXT NOT NULL,
  conv_id TEXT NOT NULL,
  model TEXT NOT NULL,
  dim INTEGER NOT NULL,
  vec BLOB NOT NULL,
  PRIMARY KEY (owner_type, owner_id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
  text, conv_id UNINDEXED, kind UNINDEXED, owner_id UNINDEXED,
  tokenize = 'unicode61 remove_diacritics 2'
);

-- Extraction/review jobs (processed locally, one at a time) ------------------------------
CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  interaction_id TEXT,
  version_id TEXT,
  kind TEXT NOT NULL,             -- review | extract | episode | embed | cast
  status TEXT NOT NULL DEFAULT 'pending', -- pending | running | done | failed | blocked | skipped
  attempts INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  payload_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_jobs_status ON jobs(status, created_at);

CREATE TABLE IF NOT EXISTS review_flags (
  id TEXT PRIMARY KEY,
  conv_id TEXT NOT NULL,
  version_id TEXT NOT NULL,
  interaction_id TEXT,
  kind TEXT NOT NULL,             -- agency | knowledge | continuity
  detail TEXT NOT NULL,
  evidence TEXT,
  source TEXT NOT NULL,           -- heuristic | llm
  status TEXT NOT NULL DEFAULT 'open', -- open | accepted | dismissed | corrected
  created_at TEXT NOT NULL,
  resolved_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_flags_ver ON review_flags(version_id, status);

CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  last_seen_at TEXT,
  revoked_at TEXT
);

-- User persona profiles (global): name + description used in the context + avatar.
CREATE TABLE IF NOT EXISTS personas (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  avatar TEXT,
  is_default INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS legacy_imports (
  legacy_id TEXT PRIMARY KEY,
  conv_id TEXT,
  imported_at TEXT NOT NULL,
  report_json TEXT
);

CREATE TABLE IF NOT EXISTS token_calibration (
  route_id TEXT PRIMARY KEY,
  ratio REAL NOT NULL,
  samples INTEGER NOT NULL,
  updated_at TEXT NOT NULL
);


    CREATE TABLE IF NOT EXISTS knowledge_events (
      id TEXT PRIMARY KEY, conv_id TEXT NOT NULL, memory_id TEXT NOT NULL,
      entity_key TEXT NOT NULL, entity_name TEXT NOT NULL, state TEXT NOT NULL,
      via TEXT, belief TEXT, evidence TEXT, source_interaction_id TEXT, source_version_id TEXT,
      created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS knowledge_events_memory ON knowledge_events(memory_id);
    CREATE INDEX IF NOT EXISTS knowledge_events_source ON knowledge_events(source_interaction_id);
    CREATE TABLE IF NOT EXISTS memory_replacements (
      conv_id TEXT NOT NULL, old_id TEXT NOT NULL, new_id TEXT NOT NULL, PRIMARY KEY(old_id,new_id)
    );
    INSERT OR IGNORE INTO memory_replacements SELECT conv_id,id,superseded_by FROM memories WHERE superseded_by IS NOT NULL;
    CREATE TABLE IF NOT EXISTS text_chunks (
      id TEXT PRIMARY KEY, conv_id TEXT NOT NULL, kind TEXT NOT NULL, owner_id TEXT NOT NULL,
      start_offset INTEGER NOT NULL, end_offset INTEGER NOT NULL, text TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS text_chunks_owner ON text_chunks(kind,owner_id);
    CREATE INDEX IF NOT EXISTS text_chunks_conv ON text_chunks(conv_id);
    CREATE TABLE IF NOT EXISTS narrative_snapshots (
      fingerprint TEXT PRIMARY KEY, conv_id TEXT NOT NULL, interaction_id TEXT NOT NULL,
      seq INTEGER NOT NULL, snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS narrative_snapshots_conv ON narrative_snapshots(conv_id,seq);
  
