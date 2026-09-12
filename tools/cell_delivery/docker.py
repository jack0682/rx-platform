"""Owned, disposable containers/volumes for two-image acceptance. No host device access."""
from __future__ import annotations
import json
import subprocess
import uuid
from pathlib import Path


class Docker:
    def __init__(self, evidence: Path):
        self.prefix = 'rx-delivery-' + uuid.uuid4().hex[:12]
        self.evidence = evidence
        self.containers: list[str] = []
        self.volumes: list[str] = []
        self.network = self.prefix + '-network'
        self.network_created = False
        self.front_network = None
        self.serial = 0

    def run(self, *args: str, capture: bool = True) -> str:
        result = subprocess.run(['docker', *map(str,args)], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f'Docker {args[:2]} failed ({result.returncode}): {result.stderr[-6000:]} {result.stdout[-2000:]}')
        return result.stdout.strip()

    def image(self, reference: str) -> dict:
        value = json.loads(self.run('image', 'inspect', reference))[0]
        if value['Os'] != 'linux':
            raise ValueError('Linux delivery image required')
        return value

    def volume(self, label: str) -> str:
        name = self.prefix + '-' + label
        self.run('volume', 'create', name)
        self.volumes.append(name)
        return name

    def make_network(self) -> None:
        self.run('network', 'create', '--internal', self.network)
        self.network_created = True

    def holder(self, image: str, mounts: list[str]) -> str:
        self.serial += 1
        name = self.prefix + f'-copy-{self.serial}'
        args = ['create','--name',name,'--network','none','--user','0','--entrypoint','/bin/true']
        for mount in mounts: args += ['-v',mount]
        self.run(*args,image)
        self.containers.append(name)
        return name

    def put(self, image: str, volume: str, source: Path, path: str = '/') -> None:
        holder = self.holder(image,[volume+':/copy'])
        # source directory contents, not the parent name, are copied into the owned volume.
        self.run('cp',str(source.resolve())+'/.',holder+':/copy'+path)

    def extract(self, image: str, volume: str, source: str, destination: Path) -> None:
        holder = self.holder(image,[volume+':/copy:ro'])
        self.run('cp',holder+':/copy/'+source.lstrip('/'),str(destination))

    def prepare_permissions(self, image: str, mounts: list[str]) -> None:
        args=['run','--rm','--network','none','--user','0','--cap-drop','ALL','--cap-add','CHOWN','--cap-add','FOWNER','--cap-add','DAC_OVERRIDE','--entrypoint','/bin/sh']
        for mount in mounts: args+=['-v',mount]
        # Only these exact temporary mount roots are writable; no host paths or devices.
        self.run(*args,image,'-c','mkdir -p /data/runtime && chown -R 10001:10001 /config /data /work && chmod 700 /config /data /work')

    def command(self, image: str, binary: str, arguments: list[str], mounts: list[str], label: str) -> str:
        args=['run','--rm','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user','10001:10001','--tmpfs','/tmp:rw,uid=10001,gid=10001,mode=700']
        for mount in mounts: args+=['-v',mount]
        output=self.run(*args,'--entrypoint',binary,image,*arguments)
        (self.evidence/(label+'.log')).write_text(output+'\n')
        return output

    def start(self, image: str, label: str, alias: str, binary: str, arguments: list[str], mounts: list[str], ports: list[str]=()) -> str:
        name=self.prefix+'-'+label
        network=self.network
        if ports:
            if self.front_network is None:
                self.front_network=self.prefix+'-terminal-network'
                self.run('network','create',self.front_network)
            network=self.front_network
        args=['create','--name',name,'--network',network,'--network-alias',alias,'--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--user','10001:10001','--tmpfs','/tmp:rw,uid=10001,gid=10001,mode=700']
        for mount in mounts: args+=['-v',mount]
        for port in ports: args+=['-p',port]
        self.run(*args,'--entrypoint',binary,image,*arguments)
        self.containers.append(name)
        if ports:
            self.run('network','connect','--alias',alias,self.network,name)
        self.run('start',name)
        return name

    def state(self, name: str) -> dict:
        return json.loads(self.run('inspect',name))[0]

    def capture_logs(self) -> None:
        for name in self.containers:
            result=subprocess.run(['docker','logs',name],capture_output=True,text=True)
            if result.stdout or result.stderr:
                (self.evidence/(name+'.log')).write_text(result.stdout+result.stderr)

    def cleanup(self) -> None:
        self.capture_logs()
        # Cleanup applies only to disposable FILE_SIMULATION acceptance processes.
        # Product shutdown is checked explicitly before this final cleanup.
        for name in reversed(self.containers):
            subprocess.run(['docker','rm','-f',name],capture_output=True)
        for volume in reversed(self.volumes):
            subprocess.run(['docker','volume','rm',volume],capture_output=True)
        if self.network_created:
            subprocess.run(['docker','network','rm',self.network],capture_output=True)
        if self.front_network:
            subprocess.run(['docker','network','rm',self.front_network],capture_output=True)
