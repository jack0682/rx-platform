"""mTLS unary calls; transport reconnect never restores runtime authority."""
import importlib
import json
from importlib.resources import files
import grpc
from google.protobuf import descriptor_pool, message_factory
from . import wire

class CompatibilityError(wire.WireError):
    def __init__(self, detail, code="FAILED_PRECONDITION"):
        super().__init__(code, detail)

class Client:
    def __init__(self, target, *, roots, certificate, private_key, server_name=None):
        if not roots or not certificate or not private_key: raise ValueError("mTLS material required")
        self.build = json.loads(files('rxclpy').joinpath('_build.json').read_text())
        for module in self.build['modules']: importlib.import_module(module)
        credentials = grpc.ssl_channel_credentials(roots, private_key, certificate)
        options = [('grpc.enable_retries', 0), ('grpc.max_send_message_length', wire.MAX_BYTES), ('grpc.max_receive_message_length', wire.MAX_BYTES)]
        if server_name: options.append(('grpc.ssl_target_name_override', server_name))
        self.channel = grpc.secure_channel(target, credentials, options=options)

    def call(self, method, request, *, timeout=10):
        """Return only the typed runtime reply. No auto-resubmit, polling or cancel."""
        path = method[1:] if method.startswith('/') else method
        try:
            service_name, method_name = path.split('/')
            service = descriptor_pool.Default().FindServiceByName(service_name)
            descriptor = service.methods_by_name[method_name]
        except (KeyError, ValueError) as error:
            raise CompatibilityError('method outside bundled unary client baseline', 'UNIMPLEMENTED') from error
        if descriptor.client_streaming or descriptor.server_streaming:
            raise CompatibilityError('streaming is outside this client baseline', 'UNIMPLEMENTED')
        if request.DESCRIPTOR.full_name != descriptor.input_type.full_name:
            raise CompatibilityError('request type differs from bundle method', 'INVALID_ARGUMENT')
        # Pre-encode outside gRPC callbacks so local wire errors retain their category.
        payload = wire.encode(request)
        wire.validate(descriptor.input_type, payload)
        response = self.channel.unary_unary('/' + service_name + '/' + method_name)(payload, timeout=timeout)
        return wire.decode(message_factory.GetMessageClass(descriptor.output_type), response)

    def open(self, hello, *, timeout=10):
        session = self.call('rx.contract.v1.SessionService/Open', hello, timeout=timeout)
        if not session.HasField('selected_version') or session.selected_version.major != 1 or session.selected_version.minor != 0 or session.selected_version.schema_hash.hex() != self.build['base_manifest_sha256']:
            raise CompatibilityError('unsupported runtime contract')
        if set(session.required_features) != {'rx.cell.v1', 'strict-wire-v1'}:
            raise CompatibilityError('unsupported or missing required features')
        return session

    def close(self):
        """Close transport only. This is not Operation.Cancel or release of ownership."""
        self.channel.close()

    def __enter__(self): return self
    def __exit__(self, *_): self.close()
