#!/usr/bin/env python3
"""Input-package refusals happen before generation; expectations are not codec goldens."""
from pathlib import Path
import json
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
class PreparationBoundary(unittest.TestCase):
    def test_missing_profile_and_tampered_file_are_refused_before_output(self):
        for mode in ['missing-profile','tampered-file']:
            with tempfile.TemporaryDirectory() as directory:
                root=Path(directory);bundle=root/'bundle'
                subprocess.run(['python3',str(ROOT/'tools/export_protocol.py'),str(bundle)],check=True,capture_output=True)
                if mode=='missing-profile':
                    path=bundle/'bundle.json';value=json.loads(path.read_text());value['files'].pop('strict-wire-v1/vectors.json');path.write_text(json.dumps(value))
                else:(bundle/'strict-wire-v1/vectors.json').write_text('{}')
                out=root/'prepared';result=subprocess.run(['python3',str(ROOT/'tools/prepare_clients.py'),'--bundle',str(bundle),'--output',str(out),'--protoc','/intentionally/unavailable'],capture_output=True,text=True)
                self.assertNotEqual(result.returncode,0)
                self.assertFalse(out.exists())
                self.assertIn('complete strict-wire-v1 bundle required' if mode=='missing-profile' else 'bundle digest mismatch',result.stderr)
    def test_clean_generator_recreates_all_rpc_idl(self):
        import shutil
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'tools').mkdir();(root/'proto').mkdir()
            shutil.copy2(ROOT/'tools/generate_protocol.py',root/'tools/generate_protocol.py')
            subprocess.run(['python3',str(root/'tools/generate_protocol.py')],check=True,capture_output=True)
            expected={p.relative_to(ROOT/'proto'):p.read_bytes() for p in (ROOT/'proto/rx').rglob('*.proto')}
            actual={p.relative_to(root/'proto'):p.read_bytes() for p in (root/'proto/rx').rglob('*.proto')}
            self.assertEqual(actual,expected)
            self.assertEqual(len(actual),9)

if __name__=='__main__':unittest.main()
