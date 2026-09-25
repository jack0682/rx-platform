#!/usr/bin/env python3
"""Prepare relocatable C++/Python client source packages from one verified bundle.

No Rust runtime or device driver is included. Existing nonempty output is refused.
The bundle's hashes establish consistency, not publisher authentication.
"""
from pathlib import Path, PurePosixPath
import argparse
import hashlib
import json
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {'rx/contract/v1/contract.proto', 'rx/cell/v1/cell.proto',
            'contracts_manifest.json', 'cell_operations_manifest.json',
            'strict-wire-v1/README.md', 'strict-wire-v1/vectors.json', 'strict-wire-v1/probe.proto'}

def manifest_bytes(value):
    # The frozen manifests are in this JCS-compatible subset. Do not silently
    # pretend the stdlib is a general-purpose JCS implementation for future input.
    def check(v):
        if isinstance(v, dict):
            for k, x in v.items():
                if not k.isascii(): raise ValueError('manifest keys require reviewed JCS support')
                check(x)
        elif isinstance(v, list):
            for x in v: check(x)
        elif isinstance(v, float) or (isinstance(v, int) and not isinstance(v, bool) and abs(v) > (1 << 53) - 1):
            raise ValueError('manifest numbers require reviewed JCS support')
    check(value)
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--protoc', default='protoc')
    a = parser.parse_args(); bundle = a.bundle.absolute(); output = a.output.absolute()
    if bundle.is_symlink() or not bundle.is_dir(): raise SystemExit('real bundle directory required')
    raw = (bundle / 'bundle.json').read_bytes(); manifest = json.loads(raw)
    if manifest.get('schema') != 'rx.protocol-source-bundle.v1' or not REQUIRED <= manifest['files'].keys():
        raise SystemExit('complete strict-wire-v1 bundle required')
    for name, expected in manifest['files'].items():
        path = PurePosixPath(name)
        if path.is_absolute() or '..' in path.parts or '\\' in name: raise SystemExit('invalid bundle path')
        full = bundle / path
        candidate = full
        while candidate != bundle.parent:
            if candidate.is_symlink(): raise SystemExit('bundle symlink refused')
            candidate = candidate.parent
        if hashlib.sha256(full.read_bytes()).hexdigest() != expected: raise SystemExit('bundle digest mismatch: '+name)
    if output.exists(): raise SystemExit('fresh output directory required')
    output.mkdir(parents=True)
    packaged_bundle = output / 'protocol'; packaged_bundle.mkdir()
    for name in ['bundle.json', *manifest['files']]:
        dest = packaged_bundle / name; dest.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(bundle / name, dest)
    for language in ['rxclpy', 'rxclcpp']:
        shutil.copytree(ROOT / 'clients' / language, output / language,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc', 'build', '*.egg-info'))
        shutil.copy2(ROOT / 'LICENSE', output / language / 'LICENSE')
        shutil.copy2(ROOT / 'NOTICE', output / language / 'NOTICE')
    package = output / 'rxclpy/src/rxclpy'
    generated = package / '_generated'; generated.mkdir()
    proto = sorted(name for name in manifest['files'] if name.endswith('.proto'))
    command = [a.protoc, '--proto_path='+str(packaged_bundle), '--python_out='+str(generated), '--pyi_out='+str(generated), *[str(packaged_bundle / name) for name in proto]]
    subprocess.run(command, check=True)
    modules = []; guards = []
    for path in [*generated.rglob('*.py'), *generated.rglob('*.pyi')]:
        text = path.read_text()
        text = re.sub(r'(?m)^from (rx(?:\.[\w]+)*) import ', r'from rxclpy._generated.\1 import ', text)
        text = re.sub(r"(_builder.BuildTopDescriptorsAndMessages\(DESCRIPTOR,\s*)'([^']+)'", r"\1'rxclpy._generated.\2'", text)
        path.write_text(text)
        if path.suffix == '.py':
            module = 'rxclpy._generated.' + '.'.join(path.relative_to(generated).with_suffix('').parts)
            (guards if 'strict_wire_v1' in module else modules).append(module)
        for parent in [path.parent, *path.parents]:
            if parent == package: break
            (parent / '__init__.py').touch()
    shutil.copytree(packaged_bundle, package / '_bundle')
    build = {'sdk_version': '0.1.0', 'profile': 'strict-wire-v1',
             'bundle_sha256': hashlib.sha256(raw).hexdigest(),
             'base_manifest_sha256': hashlib.sha256(manifest_bytes(json.loads((bundle / 'contracts_manifest.json').read_text()))).hexdigest(),
             'cell_manifest_sha256': hashlib.sha256(manifest_bytes(json.loads((bundle / 'cell_operations_manifest.json').read_text()))).hexdigest(),
             'modules': sorted(modules), 'guard_modules': sorted(guards),
             'protoc_version': subprocess.check_output([a.protoc, '--version'], text=True).strip()}
    (package / '_build.json').write_text(json.dumps(build, indent=2)+'\n')
    (package / 'py.typed').touch()
    (output / 'build.json').write_text(json.dumps(build, indent=2)+'\n')
    header = '#pragma once\n#include <string_view>\nnamespace rxclcpp::build_info {\n'
    for key in ['bundle_sha256', 'base_manifest_sha256', 'cell_manifest_sha256']:
        header += 'inline constexpr std::string_view ' + key + ' = "' + build[key] + '";\n'
    header += '}\n'
    (output / 'rxclcpp/include/rxclcpp/build_info.hpp').write_text(header)
    print(json.dumps(build))

if __name__ == '__main__': main()
