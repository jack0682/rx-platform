-- Compatibility barrier for recorded Cell mode/commissioning/block metadata and OPERATOR_API peers.
-- Existing record bytes and authority are preserved. Missing metadata is not fabricated.
-- Earlier runtimes refuse user_version > 3 before reading or writing application entities.
BEGIN IMMEDIATE;
PRAGMA user_version=4;
COMMIT;
