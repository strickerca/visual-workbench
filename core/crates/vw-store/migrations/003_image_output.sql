-- 003: persist the explicit model output-layer convention. Forward only.
-- Only objects references layers in verified v2. Copy both before dropping
-- child then parent, retaining foreign_keys=ON throughout the transaction.
CREATE TABLE layers_v3 (
  layer_id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents(document_id),
  page_index INTEGER NOT NULL DEFAULT -1,
  name TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('annotation','mask','result','adjustment','output')),
  order_key TEXT NOT NULL,
  visible INTEGER NOT NULL DEFAULT 1,
  locked INTEGER NOT NULL DEFAULT 0,
  opacity REAL NOT NULL DEFAULT 1.0 CHECK (opacity BETWEEN 0.0 AND 1.0),
  blend TEXT NOT NULL DEFAULT 'normal' CHECK (blend IN ('normal','multiply'))
);
INSERT INTO layers_v3(layer_id,document_id,page_index,name,kind,order_key,visible,locked,opacity,blend)
  SELECT layer_id,document_id,page_index,name,kind,order_key,visible,locked,opacity,blend FROM layers;
CREATE TABLE objects_v3 (
  object_id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents(document_id),
  layer_id TEXT NOT NULL REFERENCES layers_v3(layer_id),
  kind TEXT NOT NULL,
  order_key TEXT NOT NULL,
  role TEXT CHECK (role IN ('none','change','preserve','reference','explain')),
  marker_number INTEGER,
  group_id TEXT,
  state BLOB NOT NULL,
  bbox_x REAL, bbox_y REAL, bbox_w REAL, bbox_h REAL,
  deleted INTEGER NOT NULL DEFAULT 0,
  updated_seq INTEGER NOT NULL
);
INSERT INTO objects_v3(object_id,document_id,layer_id,kind,order_key,role,marker_number,group_id,state,bbox_x,bbox_y,bbox_w,bbox_h,deleted,updated_seq)
  SELECT object_id,document_id,layer_id,kind,order_key,role,marker_number,group_id,state,bbox_x,bbox_y,bbox_w,bbox_h,deleted,updated_seq FROM objects;
DROP TABLE objects;
DROP TABLE layers;
ALTER TABLE layers_v3 RENAME TO layers;
ALTER TABLE objects_v3 RENAME TO objects;
CREATE INDEX objects_by_layer ON objects(document_id,layer_id,order_key);
CREATE INDEX objects_by_marker ON objects(document_id,marker_number) WHERE marker_number IS NOT NULL;
UPDATE meta SET value='3' WHERE key='schema_version';
