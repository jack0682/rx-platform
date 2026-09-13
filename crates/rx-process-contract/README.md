# Shared process semantics model

Shares declarative ProcessSource/ResolvedProcess and bounded validation/frontier rules. It has no ROS, BT, network/filesystem, or package-loader dependencies. The S compiler and P application both use this crate. It is delivered to S as an SDK source bundle without exporting P's business authority engine.

Validates node/source identity, structural bounds, bindings, and parallel resource conflicts. Frontier accepts only complete and consistent progress. It does not reduce UNKNOWN/conflicting/unresolved states to failure and rejects history from unselected branches or missing predecessors.

This crate does not itself grant authority. P checks current authority, command identity, conditions, and checkpoints in a transaction and records the outcome. The S compiler/XML and C++ BT do not replace P's decisions or admission.
