"""Refuse an unprepared source tree instead of installing an unusable client."""
from pathlib import Path
import hashlib
import json
from setuptools import setup
from setuptools.command.build_py import build_py
from setuptools.errors import SetupError

class CheckedBuild(build_py):
    def run(self):
        root=Path(__file__).parent/'src/rxclpy'
        if not (root/'_build.json').is_file():
            raise SetupError('Use tools/prepare_clients.py with a verified protocol bundle first')
        info=json.loads((root/'_build.json').read_text());bundle=root/'_bundle'
        raw=(bundle/'bundle.json').read_bytes()
        if hashlib.sha256(raw).hexdigest()!=info['bundle_sha256']:raise SetupError('bundle identity changed')
        for path, expected in json.loads(raw)['files'].items():
            if hashlib.sha256((bundle/path).read_bytes()).hexdigest()!=expected:raise SetupError('bundle content changed: '+path)
        super().run()

setup(cmdclass={'build_py':CheckedBuild})
