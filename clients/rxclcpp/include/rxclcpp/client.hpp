#pragma once
#include "rx/contract/v1/contract.pb.h"
#include <grpcpp/grpcpp.h>
#include <chrono>
#include <memory>
#include <string>
#include <string_view>

namespace rxclcpp {
// Transport credentials only, not an operating-area signing key or permission.
struct Credentials { std::string roots, certificate, private_key; };
class Client {
 public:
  Client(std::string target, const Credentials&, std::string server_name = {});
  grpc::Status call(std::string_view method, const google::protobuf::Message& request,
                    google::protobuf::Message& response,
                    std::chrono::milliseconds timeout = std::chrono::seconds(10));
  grpc::Status open(const rx::contract::v1::PeerHello&, rx::contract::v1::Session&,
                    std::chrono::milliseconds timeout = std::chrono::seconds(10));
  // Destruction closes this transport handle; it does not issue cancellation,
  // revoke a runtime grant, delete a registration or infer an operation outcome.
 private:
  std::shared_ptr<grpc::Channel> channel_;
};
}
