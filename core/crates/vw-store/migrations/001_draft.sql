-- Visual Workbench project database — DRAFT schema 1 (2026-10-01).
-- Finalized in task T1.03. One database per project folder (<name>.vwb/project.sqlite).
-- Runtime pragmas (set on open, not stored here):
--   PRAGMA journal_mode = WAL;
--   PRAGMA synchronous = NORMAL;
--   PRAGMA foreign_keys = ON;
-- Conventions: ids are TEXT (UUIDv7 canonical form, Crockford base32 DeviceId, BLAKE3 hex AssetId);
-- times are INTEGER milliseconds since the Unix epoch; protobuf payloads use vw.v1 messages.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,           -- schema_version, project_id, created_at, created_by_app_version, ...
  value TEXT NOT NULL
);

-- Devices that have edited this project. Pairing material (keys, pinned certificates, paired and
-- revoked times) is app-level, never stored here (BUILD_SPECIFICATION §4.7). .vwbz exports strip
-- labels; fixture projects use synthetic device IDs.
CREATE TABLE devices (
  device_id   TEXT PRIMARY KEY,
  label       TEXT,                  -- user-chosen label only
  platform    TEXT NOT NULL CHECK (platform IN ('android', 'windows', 'ios')),
  lamport     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE assets (
  asset_id      TEXT PRIMARY KEY,    -- BLAKE3 of original bytes; file at blobs/<aa>/<bb>/<asset_id>
  format        TEXT NOT NULL,
  width         INTEGER,
  height        INTEGER,
  orientation   INTEGER CHECK (orientation BETWEEN 1 AND 8),
  bit_depth     INTEGER,
  has_alpha     INTEGER NOT NULL DEFAULT 0,
  color_space   TEXT,
  icc_profile   BLOB,
  byte_size     INTEGER NOT NULL,
  source        TEXT NOT NULL CHECK (source IN ('camera', 'import', 'capture', 'ai_result')),
  captured_at   INTEGER,
  metadata_json TEXT,                -- whitelisted metadata only
  local_state   TEXT NOT NULL DEFAULT 'present' CHECK (local_state IN ('present', 'remote', 'fetching'))
);

CREATE TABLE documents (
  document_id      TEXT PRIMARY KEY,
  kind             TEXT NOT NULL CHECK (kind IN ('image', 'pdf', 'svg', 'capture', 'timeline')),
  schema_version   INTEGER NOT NULL,
  title            TEXT,
  primary_asset_id TEXT REFERENCES assets (asset_id),
  created_at       INTEGER NOT NULL,
  deleted_at       INTEGER
);

-- Capture info for documents of kind 'capture' (CORE-006).
CREATE TABLE captures (
  document_id        TEXT PRIMARY KEY REFERENCES documents (document_id),
  capture_session_id TEXT NOT NULL,
  frame_id           INTEGER NOT NULL,
  geometry           BLOB NOT NULL,      -- vw.v1.CaptureGeometry
  platform           TEXT NOT NULL CHECK (platform IN ('windows', 'android')),
  app_name           TEXT,
  window_title       TEXT,               -- local only; packages include it only when the owner opts in
  lossless           INTEGER NOT NULL DEFAULT 1,
  degraded           INTEGER NOT NULL DEFAULT 0,
  captured_at        INTEGER NOT NULL
);

CREATE TABLE layers (
  layer_id    TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents (document_id),
  page_index  INTEGER NOT NULL DEFAULT -1,
  name        TEXT,
  kind        TEXT NOT NULL CHECK (kind IN ('annotation', 'mask', 'result', 'adjustment')),
  order_key   TEXT NOT NULL,
  visible     INTEGER NOT NULL DEFAULT 1,
  locked      INTEGER NOT NULL DEFAULT 0,
  opacity     REAL NOT NULL DEFAULT 1.0 CHECK (opacity BETWEEN 0.0 AND 1.0),
  blend       TEXT NOT NULL DEFAULT 'normal' CHECK (blend IN ('normal', 'multiply'))
);

CREATE TABLE objects (
  object_id     TEXT PRIMARY KEY,
  document_id   TEXT NOT NULL REFERENCES documents (document_id),
  layer_id      TEXT NOT NULL REFERENCES layers (layer_id),
  kind          TEXT NOT NULL,
  order_key     TEXT NOT NULL,
  role          TEXT CHECK (role IN ('none', 'change', 'preserve', 'reference', 'explain')),
  marker_number INTEGER,
  group_id      TEXT,
  state         BLOB NOT NULL,       -- vw.v1.ObjectState
  bbox_x        REAL, bbox_y REAL, bbox_w REAL, bbox_h REAL,  -- D-space bounds for hit testing
  deleted       INTEGER NOT NULL DEFAULT 0,
  updated_seq   INTEGER NOT NULL     -- host_seq of the last change
);
CREATE INDEX objects_by_layer ON objects (document_id, layer_id, order_key);
CREATE INDEX objects_by_marker ON objects (document_id, marker_number) WHERE marker_number IS NOT NULL;

-- Accepted transactions in host order (authoritative on the PC; replica on the phone).
CREATE TABLE op_log (
  host_seq        INTEGER PRIMARY KEY,
  txn_id          TEXT NOT NULL UNIQUE,  -- idempotency key
  device_id       TEXT NOT NULL,
  base_host_seq   INTEGER NOT NULL,
  gesture_id      TEXT,
  txn             BLOB NOT NULL,         -- vw.v1.Transaction
  state_hash      TEXT NOT NULL,         -- hex BLAKE3 after applying
  created_at_wall INTEGER NOT NULL,
  accepted_at     INTEGER NOT NULL
);

-- Phone only: local transactions not yet acknowledged by the host (offline queue).
CREATE TABLE pending_txns (
  local_seq       INTEGER PRIMARY KEY AUTOINCREMENT,
  txn_id          TEXT NOT NULL UNIQUE,
  base_host_seq   INTEGER NOT NULL,
  txn             BLOB NOT NULL,
  created_at_wall INTEGER NOT NULL
);

CREATE TABLE snapshots (
  host_seq   INTEGER PRIMARY KEY,
  state      BLOB NOT NULL,            -- canonical serialized document state
  state_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE instructions (
  instruction_id TEXT PRIMARY KEY,
  document_id    TEXT NOT NULL REFERENCES documents (document_id),
  target_ids     TEXT NOT NULL,        -- JSON array of object ids
  role           TEXT NOT NULL CHECK (role IN ('none', 'change', 'preserve', 'reference', 'explain')),
  text           TEXT NOT NULL,
  entry_method   TEXT NOT NULL CHECK (entry_method IN ('pc_keyboard', 'phone_keyboard', 'voice', 'handwriting')),
  language       TEXT,
  updated_at     INTEGER NOT NULL
);

CREATE TABLE semantic_snapshots (
  snapshot_id    TEXT PRIMARY KEY,
  document_id    TEXT NOT NULL REFERENCES documents (document_id),
  platform       TEXT NOT NULL CHECK (platform IN ('uia', 'android_ax', 'chromium_uia')),
  frame_delta_ms INTEGER,
  elements       BLOB NOT NULL,        -- zstd-compressed JSON; all text untrusted
  created_at     INTEGER NOT NULL
);

CREATE TABLE packages (
  package_id      TEXT PRIMARY KEY,
  document_id     TEXT NOT NULL REFERENCES documents (document_id),
  revision        TEXT NOT NULL,       -- r{seq}-{hash8}
  target          TEXT NOT NULL CHECK (target IN ('claude', 'openai', 'gemini', 'generic')),
  path            TEXT NOT NULL,
  manifest_sha256 TEXT NOT NULL,
  created_at      INTEGER NOT NULL,
  sent_at         INTEGER,
  sent_via        TEXT                 -- mcp_pull | claude_channel | codex_app_server | clipboard | drag | share
);

CREATE TABLE results (
  result_id                TEXT PRIMARY KEY,
  document_id              TEXT NOT NULL REFERENCES documents (document_id),
  package_id               TEXT REFERENCES packages (package_id),
  provider                 TEXT NOT NULL,
  model                    TEXT NOT NULL,
  request_json             TEXT NOT NULL,
  output_asset_id          TEXT REFERENCES assets (asset_id),
  composite_asset_id       TEXT REFERENCES assets (asset_id),
  proof_json               TEXT,       -- must include changed_outside = 0 for status ready/accepted/partial
  metrics_json             TEXT,
  cost_estimate_usd        REAL,
  status                   TEXT NOT NULL CHECK (status IN ('pending', 'ready', 'accepted', 'rejected', 'partial', 'error')),  -- mapping: BUILD_SPECIFICATION §4.18
  acceptance_mask_asset_id TEXT REFERENCES assets (asset_id),
  created_at               INTEGER NOT NULL
);

-- Written from Phase 1 when an offline or delayed edit meets a newer edit of the same property
-- (BUILD_SPECIFICATION §4.4); reviewed and swapped in the Phase 3 UI (SYNC-003).
CREATE TABLE conflicts (
  conflict_id  TEXT PRIMARY KEY,
  object_id    TEXT NOT NULL,
  property     TEXT NOT NULL,
  kept_value   BLOB,
  other_value  BLOB,
  kept_device  TEXT,
  other_device TEXT,
  delete_vs_edit INTEGER NOT NULL DEFAULT 0,  -- 1 when a delete won over an edit
  edited_object BLOB,                         -- vw.v1.ObjectState of the edited object, kept for restore when delete_vs_edit = 1
  created_at   INTEGER NOT NULL,
  resolved_at  INTEGER,
  resolution   TEXT CHECK (resolution IN ('kept', 'swapped', 'restored'))
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

INSERT INTO meta (key, value) VALUES ('schema_version', '1');
