"""strict-wire-v1 structural checks. No application admission or authority logic."""
from functools import lru_cache
import math
import struct
from google.protobuf import descriptor_pb2
from google.protobuf.descriptor import FieldDescriptor as F
from google.protobuf.message import DecodeError, EncodeError

MAX_BYTES = 1_048_576

class WireError(ValueError):
    def __init__(self, code, detail):
        self.code = code
        super().__init__(detail)

def invalid(detail):
    raise WireError('INVALID_ARGUMENT', detail)

def resource(detail):
    raise WireError('RESOURCE_EXHAUSTED', detail)

@lru_cache(maxsize=64)
def _optional_fields(serialized_file):
    file = descriptor_pb2.FileDescriptorProto.FromString(serialized_file)
    names = set()
    def walk(messages, prefix):
        for message in messages:
            full = prefix + message.name
            names.update(full + '.' + f.name for f in message.field if f.proto3_optional)
            walk(message.nested_type, full + '.')
    walk(file.message_type, file.package + '.' if file.package else '')
    return frozenset(names)

def _wire(field):
    if field.type in (F.TYPE_DOUBLE, F.TYPE_FIXED64, F.TYPE_SFIXED64): return 1
    if field.type in (F.TYPE_STRING, F.TYPE_BYTES, F.TYPE_MESSAGE): return 2
    if field.type in (F.TYPE_FLOAT, F.TYPE_FIXED32, F.TYPE_SFIXED32): return 5
    if field.type == F.TYPE_GROUP: invalid('groups unsupported')
    return 0

def validate(descriptor, data):
    if len(data) > MAX_BYTES: resource('message exceeds 1 MiB')
    budget = [65_536]
    def unit():
        if not budget[0]: resource('wire work budget exhausted')
        budget[0] -= 1
    def scan(desc, raw, depth):
        if depth >= 64: invalid('message depth limit')
        at = 0
        def varint():
            nonlocal at
            value = 0
            for shift in range(0, 70, 7):
                if at == len(raw): invalid('truncated varint')
                b = raw[at]; at += 1
                if shift == 63 and b > 1: invalid('varint overflow')
                value |= (b & 127) << shift
                if b < 128: return value
            invalid('varint overflow')
        def take(n):
            nonlocal at
            if n > len(raw) - at: invalid('truncated field')
            result = raw[at:at+n]; at += n
            return result
        def scalar(field):
            kind = _wire(field)
            if kind == 0:
                value = varint()
                if field.type == F.TYPE_BOOL and value > 1: invalid('boolean outside 0..1')
                if field.type in (F.TYPE_UINT32, F.TYPE_SINT32) and value > 0xffffffff: invalid('uint32 overflow')
                if field.type == F.TYPE_ENUM and (value == 0 or value > 0x7fffffff or value not in field.enum_type.values_by_number): invalid('unspecified or unknown enum')
            elif kind in (1, 5):
                value = take(8 if kind == 1 else 4)
                if field.type in (F.TYPE_FLOAT, F.TYPE_DOUBLE):
                    if not math.isfinite(struct.unpack('<d' if kind == 1 else '<f', value)[0]): invalid('nonfinite real')
            else:
                value = take(varint())
                if field.type == F.TYPE_STRING:
                    try: value.tobytes().decode('utf-8')
                    except UnicodeDecodeError: invalid('invalid UTF-8')
        seen = set(); groups = set()
        while at < len(raw):
            unit(); key = varint(); tag = key >> 3; wire = key & 7
            if not tag or tag > 0xffffffff: invalid('field number outside range')
            field = desc.fields_by_number.get(tag)
            if field is None: invalid('unknown field')
            if field.message_type and field.message_type.GetOptions().map_entry: invalid('unreviewed map schema')
            if not field.is_repeated:
                if tag in seen: invalid('duplicate singular field')
                seen.add(tag)
            if field.containing_oneof:
                group = field.containing_oneof.full_name
                if group in groups: invalid('multiple oneof occurrences')
                groups.add(group)
            expected = _wire(field)
            if field.is_repeated and expected != 2 and wire == 2:
                packed = take(varint()); outer_raw, outer_at = raw, at
                raw, at = packed, 0
                while at < len(raw): unit(); scalar(field)
                raw, at = outer_raw, outer_at
            else:
                if wire != expected: invalid('wrong wire type')
                if field.type == F.TYPE_MESSAGE: scan(field.message_type, take(varint()), depth + 1)
                else: scalar(field)
        optional = _optional_fields(desc.file.serialized_pb) if desc.oneofs else frozenset()
        for group in desc.oneofs:
            synthetic = len(group.fields) == 1 and group.fields[0].full_name in optional
            if not synthetic and group.full_name not in groups: invalid('missing oneof')
    scan(descriptor, memoryview(data), 0)

def encode(message):
    try: data = message.SerializeToString()
    except (EncodeError, UnicodeError, ValueError) as error: invalid(str(error))
    validate(message.DESCRIPTOR, data)
    return data

def decode(message_class, data):
    validate(message_class.DESCRIPTOR, data)
    try: return message_class.FromString(data)
    except (DecodeError, UnicodeError, ValueError) as error: invalid(str(error))
