"""Run the fixed, bundled corpus. Never derive expectations from this implementation."""
import hashlib
import importlib
import json
from importlib.resources import files
from google.protobuf import descriptor_pool, message_factory
from .wire import decode, encode, WireError

def varint(value):
    out = bytearray()
    while value >= 128: out.append((value & 127) | 128); value >>= 7
    out.append(value); return bytes(out)

def delimited(tag, body): return varint((tag << 3) | 2) + varint(len(body)) + body

def expand(value):
    if 'hex' in value: return bytes.fromhex(value['hex'])
    if 'concat' in value: return b''.join(map(expand, value['concat']))
    if 'repeat' in value:
        item=value['repeat']; return bytes.fromhex(item['hex']) * item['count']
    if 'delimited' in value:
        item=value['delimited']; return delimited(item['tag'], expand(item['body']))
    if 'nest' in value:
        item=value['nest']; result=expand(item['leaf'])
        for _ in range(item['levels']): result=delimited(item['tag'], result)
        return result
    raise ValueError('unknown corpus recipe')

def main():
    root=files('rxclpy'); build=json.loads(root.joinpath('_build.json').read_text())
    for module in build['modules']+build['guard_modules']: importlib.import_module(module)
    corpus=json.loads(root.joinpath('_bundle/strict-wire-v1/vectors.json').read_text())
    results=[]
    for case in corpus['cases']:
        cls=message_factory.GetMessageClass(descriptor_pool.Default().FindMessageTypeByName(case['message']))
        data=expand(case['input']); actual='OK'; value=None
        try: value=decode(cls,data)
        except WireError as error: actual=error.code
        row={'name':case['name'],'schema':case['schema'],'expected':case['status'],'actual':actual,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
        if 'roundtrip_status' in case:
            assert value is not None
            status='OK'
            try: encode(value)
            except WireError as error: status=error.code
            row.update(roundtrip_expected=case['roundtrip_status'],roundtrip_actual=status)
        results.append(row)
    print(json.dumps({'bundle_sha256':build['bundle_sha256'],'cases':results},indent=2))
    assert all(r['expected']==r['actual'] and r.get('roundtrip_expected')==r.get('roundtrip_actual') for r in results)

if __name__ == '__main__': main()
