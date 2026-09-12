-- Session terminal bindings change authority semantics and decoder shape.
-- Older runtimes must not ignore this binding. Legacy session bytes remain unchanged;
-- they carry no terminal proof and are invalidated by the normal Runtime boot boundary.
BEGIN IMMEDIATE;
PRAGMA user_version=5;
COMMIT;
