"""Build acceptance inputs with shipped compiler and external test-only signing helpers."""
from __future__ import annotations
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path
from .api import publish_new
from .docker import Docker


class Materials:
    def __init__(self, repository: Path, temporary: Path, evidence: Path, docker: Docker, solutions: str):
        self.repository, self.temporary, self.evidence = repository, temporary, evidence
        self.docker, self.solutions = docker, solutions
        self.seed = temporary/'seed'
        self.public_seed = temporary/'public-seed'
        self.signatures = temporary/'signatures'
        self.signatures.mkdir()
        self.sign_count = 0

    def preserve_public(self, source: Path, label: str) -> None:
        """Retain explicitly selected public inputs, never the complete temporary tree."""
        destination = self.evidence/'public-materials'/label
        destination.mkdir(parents=True, exist_ok=False)
        inventory = {}
        files = sorted(source.rglob('*')) if source.is_dir() else [source]
        for file in files:
            if file.is_symlink(): raise ValueError('public evidence must not contain symlinks')
            if not file.is_file(): continue
            relative = file.relative_to(source) if source.is_dir() else Path(file.name)
            if file.name == 'signing-fixtures.json' or file.suffix in {'.key', '.pem'}:
                raise ValueError('private or TLS material is not public evidence')
            raw = file.read_bytes()
            target = destination/relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
            inventory[str(relative)] = {'sha256':hashlib.sha256(raw).hexdigest(), 'size_bytes':len(raw)}
        publish_new(destination/'inventory.json', {'schema':'rx.delivery-public-materials.v1','files':inventory})

    def exporter(self, test: str, environment: dict[str,str], label: str) -> None:
        env = dict(os.environ, CARGO_INCREMENTAL='0', **environment)
        result = subprocess.run([str(self.repository/'tools/cargo'),'test','-p','rx-platformd','--test','delivery_fixture',test,'--locked','--offline','--','--ignored','--exact'], cwd=self.repository,env=env,capture_output=True,text=True)
        (self.evidence/(label+'.log')).write_text(result.stdout+result.stderr)
        if result.returncode or '1 passed; 0 failed' not in result.stdout:
            raise RuntimeError(f'{label} did not run exactly one successful exporter; inspect retained log')

    def sign(self, value: dict, label: str) -> Path:
        self.sign_count += 1
        request = self.signatures/f'{self.sign_count:03}-request.json'
        signature = self.signatures/f'{self.sign_count:03}-signature.json'
        publish_new(request,value)
        self.exporter('sign_delivery',{'RX_CELL_DELIVERY_SEED':str(self.seed), 'RX_CELL_SIGN_INPUT':str(request),'RX_CELL_SIGN_OUTPUT':str(signature)}, label+'-sign')
        if not signature.is_file(): raise RuntimeError('signer did not publish signature')
        # Public signature and its message metadata may be recorded; private seeds never are.
        shutil.copyfile(signature,self.evidence/(label+'.signature.json'))
        metadata=signature.with_suffix('.metadata.json')
        if metadata.exists():shutil.copyfile(metadata,self.evidence/(label+'.signing-metadata.json'))
        return signature

    def create_seed(self, architecture: str) -> None:
        self.exporter('export_delivery_seed',{'RX_CELL_DELIVERY_OUTPUT':str(self.seed),'RX_CELL_DELIVERY_ARCH':architecture},'export-seed')
        if not (self.seed/'signing-fixtures.json').is_file(): raise RuntimeError('test signing fixtures missing')
        shutil.copytree(self.seed,self.public_seed,ignore=shutil.ignore_patterns('signing-fixtures.json'))
        if list(self.public_seed.rglob('signing-fixtures.json')):raise RuntimeError('private signer leaked into compiler inputs')
        self.preserve_public(self.public_seed, 'seed')

    def compile(self) -> tuple[Path,Path,str]:
        docker=self.docker
        config=docker.volume('compiler-config');work=docker.volume('compiler-work');unused=docker.volume('compiler-data')
        docker.put(self.solutions,config,self.public_seed)
        docker.prepare_permissions(self.solutions,[config+':/config',unused+':/data',work+':/work'])
        mounts=[config+':/config:ro',work+':/work:rw']
        binary='/opt/rx/bin/rx-process-package'
        def command(args:list[str],label:str):return docker.command(self.solutions,binary,args,mounts,label)
        validator=json.loads(command(['validator-identity'],'compiler-identity'))['validator_digest']
        command(['assemble','/config/compile-input.json','/config/package-recipe.json','/work/candidate'],'package-assemble')
        key=json.loads((self.seed/'package-policy.json').read_text())['keys'][0]['id']
        command(['request','/work/candidate',key,'/work/package-signing.json'],'package-signing-request')
        request=self.temporary/'package-signing.json';docker.extract(self.solutions,work,'package-signing.json',request)
        signing=json.loads(request.read_text())
        signature=self.sign({'key':signing['key'],'message_hex':signing['message_hex']},'package')
        public=self.temporary/'public-signature';public.mkdir();shutil.copyfile(signature,public/'package.sig.json')
        (public/'package.sig.json').chmod(0o644)
        docker.put(self.solutions,config,public)
        command(['seal','/work/candidate','/config/package.sig.json','/config/package-policy.json','/work/package'],'package-seal')
        command(['compile','/work/package','/config/package-policy.json','/work/compiled'],'package-compile')
        package=self.temporary/'package';compiled=self.temporary/'compiled'
        docker.extract(self.solutions,work,'package',package);docker.extract(self.solutions,work,'compiled',compiled)
        self.preserve_public(package, 'package')
        self.preserve_public(compiled, 'compiled')
        publish_new(self.evidence/'compiled-artifacts.json',{'validator_digest':validator,'manifest_sha256':hashlib.sha256((package/'manifest.json').read_bytes()).hexdigest(),'resolved_sha256':hashlib.sha256((compiled/'resolved.json').read_bytes()).hexdigest()})
        self.compiler_config,self.compiler_work=config,work
        return package,compiled,validator

    def finalize(self, package:Path, compiled:Path, compiler_id:str, port:int, operator_bundle:Path, qualification_validator:str)->Path:
        final=self.temporary/'final'
        self.exporter('export_delivery_final',{'RX_CELL_DELIVERY_OUTPUT':str(final),'RX_CELL_DELIVERY_SEED':str(self.seed), 'RX_CELL_RESOLVED':str(compiled/'resolved.json'),'RX_CELL_PACKAGE':str(package),'RX_CELL_COMPILER_ID':compiler_id,'RX_CELL_DELIVERY_PORT':str(port),'RX_CELL_OPERATOR_BUNDLE':str(operator_bundle),'RX_CELL_QUALIFICATION_VALIDATOR_ID':qualification_validator},'export-final')
        if not (final/'config/startup.json').is_file():raise RuntimeError('final product startup missing')
        return final
