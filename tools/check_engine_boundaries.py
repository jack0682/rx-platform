#!/usr/bin/env python3
"""Check engine module references against the declared group boundaries, not behavior.

Every module under crates/rx-application/src/engine belongs to exactly one group in
docs/engine-boundaries.json. A reference from a file in one group to a module of a
group it may not depend on is a violation unless the map lists it as a known seam.
The analysis is textual: it resolves `super::`, `crate::engine::` and bare module
paths that reach an engine module through the `use super::*` chain, outside
`#[cfg(test)]` items. It does not type-check and does not follow re-exports.
"""
from collections import Counter, defaultdict
import json
from pathlib import Path
import re
import sys

from check_invariant_traceability import require, rust_code

ROOT = Path(__file__).resolve().parents[1]
MAP = ROOT / "docs/engine-boundaries.json"
ENGINE = ROOT / "crates/rx-application/src/engine"
IDENT = r"[A-Za-z_][A-Za-z0-9_]*"


def strip_test_items(code):
    """Blank every item that follows a `#[cfg(test)]` attribute, keeping line numbers."""
    result = list(code)
    for attribute in re.finditer(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]", code):
        i = attribute.end()
        depth = 0
        while i < len(code):
            c = code[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            elif c == ";" and depth == 0:
                break
            i += 1
        require(i < len(code), "unterminated item after #[cfg(test)]")
        for j in range(attribute.start(), i + 1):
            if result[j] != "\n":
                result[j] = " "
    return "".join(result)


def expand_use(tree):
    """Flatten `a::{b, c::{d, e}}` into ['a::b', 'a::c::d', 'a::c::e']."""
    tree = tree.strip()
    if tree.startswith("{") and tree.endswith("}"):
        paths, depth, start = [], 0, 1
        for i, c in enumerate(tree):
            depth += (c == "{") - (c == "}")
            if (c == "," and depth == 1) or (c == "}" and depth == 0):
                if tree[start:i].strip():
                    paths.extend(expand_use(tree[start:i]))
                start = i + 1
        return paths
    head, _, tail = tree.partition("::{")
    if tail:
        return [head.strip() + "::" + rest for rest in expand_use("{" + tail)]
    return [re.sub(r"\s+as\s+\w+$", "", tree)]


def references(path, modules, submodules):
    """Return ((line, engine module) ...) for every resolvable engine reference in `path`."""
    relative = path.relative_to(ENGINE)
    own = relative.with_suffix("").parts
    if own[-1] == "mod":
        own = own[:-1]
    top = own[0] if own else "mod"
    code = strip_test_items(rust_code(path.read_text(encoding="utf-8")))
    found, glob = [], False

    def resolve(segments):
        if not segments:
            return None
        if segments[0] == "crate":
            if len(segments) > 2 and segments[1] == "engine" and segments[2] in modules:
                return segments[2]
            return None
        if segments[0] == "self":
            return None
        module = list(own)
        while segments and segments[0] == "super":
            require(module, f"{relative}: `super` above the engine root")
            module.pop()
            segments = segments[1:]
        if not segments or segments[0] == "*":
            return None
        if module and segments[0] in submodules.get(tuple(module), ()):
            return module[0]
        # Names from the engine root reach every descendant through `use super::*`.
        return segments[0] if segments[0] in modules else None

    def record(offset, segments):
        target = resolve([s.strip() for s in segments])
        if target is not None and target != top:
            found.append((code.count("\n", 0, offset) + 1, target))

    def blank(match):
        return "".join("\n" if c == "\n" else " " for c in match.group(0))

    def use_statement(match):
        nonlocal glob
        for flat in expand_use(match.group(1)):
            glob |= flat == "super::*"
            record(match.start(), flat.split("::"))
        return blank(match)

    code = re.sub(r"\buse\s+([^;]+);", use_statement, code)
    for match in re.finditer(rf"(?<![A-Za-z0-9_])((?:{IDENT}\s*::\s*)+)(\*|{IDENT}|\{{)", code):
        segments = [s for s in re.split(r"\s*::\s*", match.group(1)) if s]
        record(match.start(), segments + [match.group(2)])
    return found, glob


def declared_modules(path):
    """Return the `mod x;` names declared in a Rust file, outside test items."""
    code = strip_test_items(rust_code(path.read_text(encoding="utf-8")))
    return re.findall(rf"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+({IDENT})\s*;", code)


def module_file(directory, name):
    for candidate in (directory / f"{name}.rs", directory / name / "mod.rs"):
        if candidate.is_file():
            return candidate
    raise ValueError(f"module {name} declared in {directory.relative_to(ROOT)} has no source file")


def engine_tree():
    """Map each top-level engine module to its files and record every submodule name."""
    files, submodules = {"mod": [ENGINE / "mod.rs"]}, {}
    pending = []
    for name in declared_modules(ENGINE / "mod.rs"):
        pending.append((name, (name,), module_file(ENGINE, name)))
    while pending:
        top, module, path = pending.pop()
        files.setdefault(top, []).append(path)
        children = declared_modules(path)
        submodules[module] = set(children)
        directory = path.parent if path.name == "mod.rs" else path.with_suffix("")
        for child in children:
            pending.append((top, module + (child,), module_file(directory, child)))
    known = {p for paths in files.values() for p in paths}
    stray = sorted(p.relative_to(ROOT) for p in ENGINE.rglob("*.rs") if p not in known)
    require(not stray, f"engine sources not reachable from mod.rs: {stray}")
    return files, submodules


def load_map():
    data = json.loads(MAP.read_text(encoding="utf-8"))
    require(data["schema"] == "rx.engine-boundaries.v1", "unknown map schema")
    for field in ("scope", "rule", "analysis_limit"):
        require(isinstance(data[field], str) and data[field].strip(), f"missing {field}")
    groups = data["groups"]
    require(isinstance(groups, dict) and groups, "groups must be a non-empty object")
    owner = {}
    for name, group in groups.items():
        require(isinstance(group["description"], str) and group["description"].strip(), f"{name}: missing description")
        allowed = group["may_reference"]
        require(isinstance(allowed, list) and name in allowed and set(allowed) <= groups.keys(), f"{name}: may_reference must name known groups including itself")
        require(isinstance(group["modules"], list) and group["modules"], f"{name}: modules must be a non-empty list")
        for module in group["modules"]:
            require(isinstance(module, str) and module not in owner, f"module assigned twice: {module}")
            owner[module] = name
    seams = data["allowed_violations"]
    require(isinstance(seams, list), "allowed_violations must be a list")
    allow = {}
    for seam in seams:
        key = (seam["file"], seam["module"])
        require(all(isinstance(k, str) for k in key), "seam file/module must be strings")
        require(key not in allow, f"duplicate seam: {key}")
        require(type(seam["references"]) is int and seam["references"] > 0, f"{key}: references must be a positive integer")
        require(isinstance(seam["reason"], str) and seam["reason"].strip(), f"{key}: missing reason")
        allow[key] = seam["references"]
    return data, owner, allow


def main():
    argv = sys.argv[1:]
    require(argv in ([], ["--graph"]), "usage: check_engine_boundaries.py [--graph]")
    data, owner, allow = load_map()
    files, submodules = engine_tree()
    modules = set(files)
    unassigned = sorted(modules - owner.keys())
    unknown = sorted(owner.keys() - modules)
    require(not (unassigned or unknown), f"module/group mismatch: unassigned={unassigned}, unknown={unknown}")
    graph, globs, edges = defaultdict(Counter), 0, 0
    violations, used = {}, set()
    for module, paths in sorted(files.items()):
        for path in sorted(paths):
            found, glob = references(path, modules, submodules)
            globs += glob
            relative = str(path.relative_to(ROOT))
            for line, target in found:
                graph[module][target] += 1
                edges += 1
                if owner[target] not in data["groups"][owner[module]]["may_reference"]:
                    entry = violations.setdefault((relative, target), [module, 0, line])
                    entry[1] += 1
    if argv == ["--graph"]:
        for module in sorted(graph):
            targets = ", ".join(f"{t}({n})" for t, n in sorted(graph[module].items()))
            print(f"{owner[module]:15} {module} -> {targets}")
    stale = sorted(allow.keys() - violations.keys())
    require(not stale, f"allowed_violations no longer occur and must be removed: {stale}")
    failures = 0
    for (relative, target), (module, count, line) in sorted(violations.items()):
        recorded = allow.get((relative, target))
        if recorded == count:
            continue
        failures += 1
        status = "not listed in the map" if recorded is None else f"map records {recorded}"
        print(f"{relative}:{line}: {owner[module]} module {module} references {target} ({owner[target]}) {count}x, {status}", file=sys.stderr)
    require(not failures, f"{failures} engine boundary violation(s) differ from docs/engine-boundaries.json")
    print(
        f"Engine boundaries: modules={len(modules)} files={sum(len(p) for p in files.values())} "
        f"glob_imports={globs} references={edges}; groups={len(data['groups'])}; "
        f"known seams={len(allow)} ({sum(allow.values())} references, may only shrink). Type resolution not checked."
    )


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(f"engine boundaries: {error}", file=sys.stderr)
        raise SystemExit(1)
