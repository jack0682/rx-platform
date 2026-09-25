#!/usr/bin/env python3
"""Test-only opaque TCP tunnel. No TLS keys, decoding, signing or authority logic.

The parent arms drop-return after negotiation, observes the real Host effect,
then cuts the connection. Reconnection requires an explicit caller action.
"""
import asyncio
import json
from pathlib import Path

control=Path('/control')
connections=set()
stats={'accepted':0,'forwarded_to_server':0,'discarded_from_server':0,'cuts':0}
def mode():
    try:return json.loads((control/'mode.json').read_text())['mode']
    except FileNotFoundError:return 'pass'
def report():
    path=control/'stats.pending';path.write_text(json.dumps(stats));path.replace(control/'stats.json')
async def handle(reader,writer):
    upstream,send=await asyncio.open_connection('p',7443);connections.add((writer,send));stats['accepted']+=1;report()
    async def copy(source,target,returning):
        while data:=await source.read(65536):
            if returning and mode()=='drop-return':stats['discarded_from_server']+=len(data);report();continue
            if not returning:stats['forwarded_to_server']+=len(data);report()
            target.write(data);await target.drain()
    tasks=[asyncio.create_task(copy(reader,send,False)),asyncio.create_task(copy(upstream,writer,True))]
    try:await asyncio.wait(tasks,return_when=asyncio.FIRST_COMPLETED)
    finally:
        for task in tasks:task.cancel()
        await asyncio.gather(*tasks,return_exceptions=True)
        connections.discard((writer,send));writer.close();send.close()
async def watch():
    while True:
        if mode()=='cut':
            for writer,send in list(connections):writer.close();send.close()
            if connections:stats['cuts']+=1;report()
        await asyncio.sleep(.01)
async def main():
    report();server=await asyncio.start_server(handle,'0.0.0.0',7443)
    async with server:await asyncio.gather(server.serve_forever(),watch())
asyncio.run(main())
