#pragma once
#include <google/protobuf/descriptor.h>
#include <google/protobuf/message.h>
#include <string>
#include <string_view>

namespace rxclcpp::wire {
enum class Code { OK, INVALID_ARGUMENT, RESOURCE_EXHAUSTED };
struct Status {
  Code code = Code::OK;
  std::string detail;
  bool ok() const noexcept { return code == Code::OK; }
};
// Structural profile only. No admission, authority issuance or device ownership.
Status validate(const google::protobuf::Descriptor&, std::string_view);
Status parse(google::protobuf::Message&, std::string_view);
Status serialize(const google::protobuf::Message&, std::string&);
const char* name(Code) noexcept;
}
