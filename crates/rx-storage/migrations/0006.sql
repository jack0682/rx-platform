-- Older runtimes must not ignore a durable process-stop admission barrier.
BEGIN IMMEDIATE;
PRAGMA user_version=6;
COMMIT;
