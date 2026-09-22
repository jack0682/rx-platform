# Invariant traceability

The [machine-readable map](invariant-traceability.json) answers which existing local test declarations have been explicitly linked to each of the 30 contract IDs. It contains both I01-I12 and OI01-OI18, including entries with no declared link.

This revision declares scenario evidence for **10/30 IDs** and leaves **20/30 uncovered**. `uncovered` means no reviewed test binding is declared here; it does not prove that relevant tests are absent. `declared` means only the scenario and boundary stated in that entry. It does not mean exhaustive invariant conformance, formal verification or physical qualification. Individual test links carry a source-reading rationale.

## What the checker establishes

```sh
python3 tools/check_invariant_traceability.py
```

The required repository CI job checks local ID completeness, named test-declaration existence, and absence of duplicate or unknown IDs. It also rejects inconsistent map status/counts and empty rationales. It reads ID definitions only in section 8 of `spec/contracts/v1.0/01_responsibility_and_semantics.md` and section 1 of `spec/cell_operations/v1.0/05_validation_audit.md`. References elsewhere in those documents are not definitions.

**The checker does not determine whether a named test actually establishes its invariant.** That relationship and its sufficiency remain a review judgment. It does not execute Rust tests, evaluate their assertions, resolve module inclusion or cfg/ignore selection, or prove test reachability. The existing Rust CI job remains responsible for executing its configured test suite. The static check recognizes ordinary `#[test]` and `#[tokio::test]` function declarations and masks comments and literals; macro-generated test names are outside this map's supported declaration form.

## Reading a specific answer

```sh
python3 - <<'PYTHON'
import json
from pathlib import Path
value = json.loads(Path("docs/invariant-traceability.json").read_text())
print(next(entry for entry in value["invariants"] if entry["id"] == "OI07"))
PYTHON
```

OI07 is **uncovered** in this repository. Generic controller ownership/reservation and adapter shutdown tests do not stage two material-support releases that rely on reciprocal current-support PASS. Naming those tests as complete material-support evidence would overstate their assertions. Other intentionally undeclared entries also explain the missing reviewed connection. No test or production behavior was changed to fill this map.

## Source and maintenance limits

The two code repositories have separate maps; each answers only for its own test declarations. A cross-repository union requires reading both entries and their stated boundaries. Adding their declaration counts would double-count shared IDs and still would not establish whole-system conformance.

The two local definition files matched their originals and the other vendored copy byte-for-byte on 2026-09-18. However, `05_validation_audit.md` is **outside the four normative documents covered by the cell manifest**. It may drift independently in a future vendored copy. This checker does not compare repositories or establish that this non-manifest document is still current.

When editing a declaration, read the actual test assertions, record their limited behavior in the rationale, and retain `uncovered` if the relation cannot be justified. Keep the map summary and this revision's counts aligned. Renaming or removing a referenced test requires a deliberate map review. A green structural check cannot approve a semantic claim.
