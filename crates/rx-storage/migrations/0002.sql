BEGIN IMMEDIATE;
ALTER TABLE outbox RENAME TO outbox_v1;
CREATE TABLE outbox (
  id TEXT PRIMARY KEY NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('NEW','EMIT_ENTERED','VOIDED','DELIVERED')),
  document BLOB NOT NULL
) STRICT;
INSERT INTO outbox(id,state,document) SELECT id,state,document FROM outbox_v1;
DROP TABLE outbox_v1;
CREATE INDEX outbox_state ON outbox(state,id);
PRAGMA user_version=2;
COMMIT;
