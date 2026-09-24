# Isolated Linux ownership probe

This standalone crate is an opt-in measurement fixture, excluded from product
workspace builds and SDK export. Its controlled single-thread fork and Command
pre-exec instrumentation uses unsafe libc calls; product crates retain their
unsafe-code prohibition. It does not introduce a shipped executable or daemon.

Run the parent `tools/storage_lock_passage.py` with a matching solutions checkout,
an existing immutable runtime image, a Rust builder and a fresh evidence path.
That procedure records commands, artifact hashes, positive release and opposite
live-writer refusals, and every unfiltered Host stress round.

The fixture uses owned, unreaped children and a fixture-only subreaper to clean up
an intentionally killed manager's descendant. Those operations are not product
process adoption or physical handover. The SIGKILL case deliberately retains the
support limit until the inherited description disappears. The injected child
transaction panic is expected and labeled; its parent transaction must survive.
