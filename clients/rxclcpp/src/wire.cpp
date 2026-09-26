// Adapted from the RX native protocol validator; canonical client implementation.
#include "rxclcpp/wire.hpp"
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <stdexcept>
#include <unordered_set>

namespace rxclcpp::wire {
namespace {
using F = google::protobuf::FieldDescriptor;
struct BadWire : std::runtime_error {
  Code code;
  explicit BadWire(const std::string& message, Code value = Code::INVALID_ARGUMENT)
      : std::runtime_error(message), code(value) {}
};
void utf8(std::string_view data) {
  std::size_t i = 0;
  while (i < data.size()) {
    const auto first = static_cast<unsigned char>(data[i++]);
    if (first < 0x80) continue;
    unsigned count; std::uint32_t cp; std::uint32_t minimum;
    if (first >= 0xc2 && first <= 0xdf) { count=1; cp=first&31; minimum=0x80; }
    else if (first >= 0xe0 && first <= 0xef) { count=2; cp=first&15; minimum=0x800; }
    else if (first >= 0xf0 && first <= 0xf4) { count=3; cp=first&7; minimum=0x10000; }
    else throw BadWire("invalid UTF-8 leading byte");
    if (count > data.size()-i) throw BadWire("incomplete UTF-8 sequence");
    for (unsigned n=0; n<count; ++n) {
      const auto next = static_cast<unsigned char>(data[i++]);
      if ((next&0xc0) != 0x80) throw BadWire("invalid UTF-8 continuation");
      cp=(cp<<6)|(next&63);
    }
    if (cp < minimum || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff))
      throw BadWire("invalid UTF-8 scalar");
  }
}
std::uint64_t varint(std::string_view bytes, std::size_t& at) {
  std::uint64_t value = 0;
  for (int shift=0; shift<=63; shift+=7) {
    if (at == bytes.size()) throw BadWire("truncated varint");
    const auto byte = static_cast<unsigned char>(bytes[at++]);
    if (shift == 63 && byte > 1) throw BadWire("varint overflow");
    value |= std::uint64_t(byte & 0x7f) << shift;
    if (!(byte & 0x80)) return value;
  }
  throw BadWire("varint overflow");
}
std::string_view take(std::string_view bytes, std::size_t& at, std::uint64_t length) {
  if (length > bytes.size()-at) throw BadWire("truncated field");
  auto out = bytes.substr(at, static_cast<std::size_t>(length));
  at += static_cast<std::size_t>(length);
  return out;
}
std::string_view length_delimited(std::string_view bytes, std::size_t& at) {
  const auto length = varint(bytes, at);
  return take(bytes, at, length);
}
int wire_type(const F& field) {
  switch (field.type()) {
    case F::TYPE_DOUBLE: case F::TYPE_FIXED64: case F::TYPE_SFIXED64: return 1;
    case F::TYPE_STRING: case F::TYPE_BYTES: case F::TYPE_MESSAGE: return 2;
    case F::TYPE_FLOAT: case F::TYPE_FIXED32: case F::TYPE_SFIXED32: return 5;
    case F::TYPE_GROUP: throw BadWire("groups unsupported");
    default: return 0;
  }
}
void scalar(const F& field, std::string_view bytes, std::size_t& at) {
  switch (wire_type(field)) {
    case 0: {
      const auto v = varint(bytes, at);
      if (field.type()==F::TYPE_BOOL && v>1) throw BadWire("invalid boolean");
      if ((field.type()==F::TYPE_UINT32 || field.type()==F::TYPE_SINT32) && v>UINT32_MAX) throw BadWire("uint32 overflow");
      if (field.type()==F::TYPE_ENUM &&
          (v==0 || v>INT32_MAX || !field.enum_type()->FindValueByNumber(static_cast<int>(v)))) throw BadWire("unknown enum");
      break;
    }
    case 1: {
      const auto raw=take(bytes,at,8);
      std::uint64_t bits=0;
      for (unsigned i=0;i<8;++i) bits|=std::uint64_t(static_cast<unsigned char>(raw[i]))<<(8*i);
      double v;
      std::memcpy(&v,&bits,8);
      if (field.type()==F::TYPE_DOUBLE && !std::isfinite(v)) throw BadWire("nonfinite double");
      break;
    }
    case 5: {
      const auto raw=take(bytes,at,4);
      std::uint32_t bits=0;
      for (unsigned i=0;i<4;++i) bits|=std::uint32_t(static_cast<unsigned char>(raw[i]))<<(8*i);
      float v;
      std::memcpy(&v,&bits,4);
      if (field.type()==F::TYPE_FLOAT && !std::isfinite(v)) throw BadWire("nonfinite float");
      break;
    }
    case 2: {
      const auto value=length_delimited(bytes,at);
      if (field.type()==F::TYPE_STRING) utf8(value);
      break;
    }
  }
}
void scan(const google::protobuf::Descriptor& desc, std::string_view bytes, unsigned depth, std::size_t& budget) {
  if (depth>=64) throw BadWire("nesting limit");
  std::size_t at=0;
  std::unordered_set<int> seen;
  std::unordered_set<const google::protobuf::OneofDescriptor*> groups;
  while (at<bytes.size()) {
    if (budget==0) throw BadWire("field limit", Code::RESOURCE_EXHAUSTED);
    --budget;
    const auto key=varint(bytes,at);
    const auto tag=key>>3;
    const auto wire=static_cast<int>(key&7);
    if (tag==0 || tag>INT32_MAX) throw BadWire("invalid tag");
    const auto* field=desc.FindFieldByNumber(static_cast<int>(tag));
    if (!field) throw BadWire("unknown field");
    if (field->is_map()) throw BadWire("unreviewed map schema");
    if (!field->is_repeated() && !seen.insert(field->number()).second) throw BadWire("duplicate singular");
    if (field->containing_oneof() && !groups.insert(field->containing_oneof()).second) throw BadWire("multiple oneof values");
    const auto expected=wire_type(*field);
    if (field->is_repeated() && expected!=2 && wire==2) {
      const auto packed=length_delimited(bytes,at);
      std::size_t pos=0;
      while(pos<packed.size()) {
        if (budget==0) throw BadWire("packed value limit", Code::RESOURCE_EXHAUSTED);
        --budget;
        scalar(*field,packed,pos);
      }
    } else {
      if (wire!=expected) throw BadWire("wrong wire type");
      if (field->type()==F::TYPE_MESSAGE) scan(*field->message_type(),length_delimited(bytes,at),depth+1,budget);
      else scalar(*field,bytes,at);
    }
  }
  for (int i=0;i<desc.oneof_decl_count();++i) {
    const auto* group=desc.oneof_decl(i);
    if (!group->is_synthetic() && !groups.contains(group)) throw BadWire("missing oneof");
  }
}
}
Status validate(const google::protobuf::Descriptor& descriptor, std::string_view bytes) {
  try {
    if(bytes.size()>1'048'576) throw BadWire("message exceeds 1 MiB", Code::RESOURCE_EXHAUSTED);
    std::size_t budget=65'536;
    scan(descriptor,bytes,0,budget);
    return {};
  } catch (const BadWire& e) { return {e.code, e.what()}; }
}
Status parse(google::protobuf::Message& message, std::string_view bytes) {
  message.Clear();
  const auto checked=validate(*message.GetDescriptor(),bytes);
  if (!checked.ok()) return checked;
  if(!message.ParseFromArray(bytes.data(),static_cast<int>(bytes.size()))) {
    message.Clear();
    return {Code::INVALID_ARGUMENT, "protobuf parse failed"};
  }
  return {};
}
Status serialize(const google::protobuf::Message& message, std::string& bytes) {
  bytes.clear();
  if (!message.SerializeToString(&bytes)) return {Code::INVALID_ARGUMENT, "protobuf serialization failed"};
  const auto checked=validate(*message.GetDescriptor(),bytes);
  if (!checked.ok()) bytes.clear();
  return checked;
}
const char* name(Code code) noexcept {
  switch(code) {
    case Code::OK: return "OK";
    case Code::INVALID_ARGUMENT: return "INVALID_ARGUMENT";
    case Code::RESOURCE_EXHAUSTED: return "RESOURCE_EXHAUSTED";
  }
  return "INVALID_ARGUMENT";
}
}
