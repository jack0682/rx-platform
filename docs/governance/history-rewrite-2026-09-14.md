# Signed history migration — 2026-09-14

The project owner authorized a history rewrite to add matching Developer
Certificate of Origin sign-offs and OpenPGP signatures to every commit reachable
from the maintained branches and archival tags. This migration covers 21
commits and 5 annotated tags in this repository.

[The migration map](history-rewrite-2026-09-14.json) records each original commit,
its replacement, unchanged tree, original parents and author, plus the old and
new branch/tag references. Author identity and author date are preserved. Parent
order and merge topology are preserved through the mapping. Commit messages
retain their original content with the missing author/owner sign-off trailers
added. The owner is the new committer and signs each replacement. The registered
OpenPGP fingerprint is `7527E36F9EB3667236B5B0919C9FE74DE196CD5A`.

Tag annotations retain their meaning and point at the corresponding rewritten
commits; the owner re-signs each annotated tag. A verified Git bundle containing
the original objects is retained outside the public repository. Its SHA-256 is
`42ead09bab69af286d9a8107630bfae40edcd8046c86ba29fd38ad9998ac1eb8`. No private key is stored in a repository or CI.

## Updating an existing checkout

A fresh clone is the simplest way to adopt this history. Keep the old checkout
until local changes and unpublished branches are accounted for. Copy only your
uncommitted work into a new feature branch based on the new `develop`.

For an existing checkout, first back up local changes and branches, then fetch
updated branches and tags. Rebase or cherry-pick your own unpublished changes onto
the new `develop`. Do not merge the old branch history into the new history, and
do not force-push `main` or `develop`. Replacing local branch pointers or tags
requires care because those references now identify different objects; the map
allows you to check their exact correspondence before making that change.

## Historical evidence

The migration changes Git object identities and signatures, not product files,
contracts, manifests, SDK exports or runtime behavior. Original SHA references
inside historical evidence retain their original meaning; use the mapping to
find the replacement with the same tree. This record is not new test evidence
for historical runtime claims.

Closed pull requests and their old DCO/check results are historical GitHub
records and are not rewritten. All newly maintained branch/tag history is
subject to the current contribution policy. A signed migration does not change
the Apache-2.0 license or erase third-party attribution.
