# Local repository ownership

`SqliteRepository` owns a private SQLite connection and a non-cloneable
`ExclusiveFileLock`. A live repository rejects a second writer. Lock acquisition
is nonblocking and happens once; a failed contender never obtains an unlocking
guard. Newly created Unix lock files are owner-only (0600, subject to umask);
existing file permissions are not rewritten.

`close(self)` consumes the repository, confirms `Connection::close()`, then
explicitly unlocks the ownership file description. Normal Drop uses the same
order. The old declaration order already closed the parent connection first;
the correction is explicit release of the shared file description even when a
fork child retains a descriptor, not a reversal of parent drop order.

If SQLite close fails, the connection and this process's lock descriptor are
intentionally retained until process exit. There is no active quarantine service
and no automatic retry. A dependency panic before close confirmation also cannot
run an unlocking guard destructor. Explicit close returns a typed
`StoreError::Ownership` failure; Drop cannot return an error. An unlock failure
is unconfirmed and retains the descriptor without retry. Inherited copies can
extend the lock's lifetime beyond the creator's exit.

Rust cannot prevent fork from copying a non-cloneable value. Runtime creator-PID
checks prevent an ordinary fork child from using or releasing an inherited
repository. The PID is ephemeral process identity within the same PID namespace,
not a persisted recovery capability or an authenticated process identity.
Inherited connection/transaction objects are not passed into SQLite destructors;
their resources disappear at exec/exit. Children must not reuse inherited SQLite
state. This is not general permission to run SQLite in arbitrary post-fork code.

A `transact` callback can also fork. All borrowed Transaction entrypoints check
the creator, and commit/rollback cleanup is creator-bound. A child's Ok, Err or
unwind cannot commit or roll back the parent's transaction. Normal parent errors
and unwind still roll back while the repository keeps exclusive ownership.

`ExclusiveFileLock` exposes no File/FD and no Clone. Its generic use by Host does
not itself know when an adapter is safe to drop; Host keeps the guard private
through its existing service/maintenance scope. It grants no physical authority.

## Abrupt loss remains a refusal boundary

SIGKILL/abort does not execute Drop or LOCK_UN. If an inherited description remains
in a pre-exec child, a new writer is refused as typed `Contended` until that
description closes at exec/exit. G1 does not remove this abrupt-loss refusal; it
bounds the window by the inherited description's lifetime, not a timeout. No PID
adoption, lock-file deletion or arbitrary retry is introduced.

The supported mechanism is cooperating processes on the tested local Linux
filesystem/runtime. Advisory locks do not isolate malicious code, filesystem/OS
replacement, remote filesystems or physical resources.

## Compatibility and verification

`OwnershipFailure` and `OwnershipError` are new Rust port types, and
`StoreError::Ownership` requires downstream exhaustive matches to be updated.
The SDK is regenerated from this source. SQLite schema/version, persisted
Document formats and wire/protobuf schemas are unchanged. HTTP/gRPC storage
failures retain their unavailable response semantics; typed details are local
and diagnostic.

`tests/ownership.rs` verifies live-writer refusal, failed contenders, rollback,
unwind and legitimate close. An internal subprocess test uses a real unfinished
SQLite statement to verify close failure does not unlock. The isolated Linux
fixture in `tools/lock_lifetime_probe` additionally tests inherited cleanup,
active transactions, real Command pre-exec windows and manager SIGKILL. That
fixture's narrowly scoped unsafe fork instrumentation is not part of product
crates or the exported SDK.

Kernel meaning: [flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html),
[fork(2)](https://man7.org/linux/man-pages/man2/fork.2.html), and
[Rust File::unlock](https://doc.rust-lang.org/std/fs/struct.File.html#method.unlock).
Measurements and scope are recorded with the compatible three-repository commits.
