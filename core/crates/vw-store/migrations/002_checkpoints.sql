-- 002: sequencing checkpoints and complete projection data. Forward only.
ALTER TABLE snapshots ADD COLUMN checkpoint BLOB;
ALTER TABLE snapshots ADD COLUMN checkpoint_hash TEXT;
ALTER TABLE op_log ADD COLUMN ack BLOB;
ALTER TABLE assets ADD COLUMN definition BLOB;
ALTER TABLE conflicts ADD COLUMN record BLOB;
ALTER TABLE documents ADD COLUMN state BLOB;
ALTER TABLE semantic_snapshots ADD COLUMN capture_session_id TEXT;
ALTER TABLE semantic_snapshots ADD COLUMN frame_id TEXT;
-- Full uint64 frame identity is canonical decimal TEXT, never a lossy REAL.
CREATE TABLE captures_v2 (
 document_id TEXT PRIMARY KEY REFERENCES documents(document_id),
 capture_session_id TEXT NOT NULL, frame_id TEXT NOT NULL, geometry BLOB NOT NULL,
 platform TEXT NOT NULL CHECK(platform IN ('windows','android')),
 app_name TEXT, window_title TEXT, lossless INTEGER NOT NULL DEFAULT 1,
 degraded INTEGER NOT NULL DEFAULT 0, captured_at INTEGER NOT NULL);
INSERT INTO captures_v2 SELECT document_id,capture_session_id,CAST(frame_id AS TEXT),
 geometry,platform,app_name,window_title,lossless,degraded,captured_at FROM captures;
DROP TABLE captures;
ALTER TABLE captures_v2 RENAME TO captures;
CREATE TABLE groups (group_id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(document_id), parent_id TEXT);
CREATE TABLE pdf_pages (document_id TEXT NOT NULL REFERENCES documents(document_id), page_index INTEGER NOT NULL, state BLOB NOT NULL, PRIMARY KEY(document_id,page_index));
CREATE TABLE mask_versions (object_id TEXT PRIMARY KEY, state BLOB NOT NULL);
-- Package identities can originate on another peer before its local package file.
CREATE TABLE results_v2 (
 result_id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(document_id),
 package_id TEXT, provider TEXT NOT NULL, model TEXT NOT NULL, request_json TEXT NOT NULL,
 output_asset_id TEXT REFERENCES assets(asset_id), composite_asset_id TEXT REFERENCES assets(asset_id),
 proof_json TEXT, metrics_json TEXT, cost_estimate_usd REAL,
 status TEXT NOT NULL CHECK(status IN ('pending','ready','accepted','rejected','partial','error')),
 acceptance_mask_asset_id TEXT REFERENCES assets(asset_id), created_at INTEGER NOT NULL);
INSERT INTO results_v2 SELECT * FROM results;
DROP TABLE results;
ALTER TABLE results_v2 RENAME TO results;
UPDATE meta SET value='2' WHERE key='schema_version';
