#include "rxclcpp/client.hpp"
#include "rxclcpp/build_info.hpp"
#include "rxclcpp/wire.hpp"
#include <grpcpp/generic/generic_stub.h>
#include <set>
#include <vector>
#include <stdexcept>

namespace rxclcpp {
namespace {
grpc::Status wire_status(const wire::Status& status) {
  return {status.code == wire::Code::RESOURCE_EXHAUSTED ? grpc::StatusCode::RESOURCE_EXHAUSTED : grpc::StatusCode::INVALID_ARGUMENT, status.detail};
}
std::string hex(std::string_view value) {
  static constexpr char digits[]="0123456789abcdef"; std::string out;
  for(unsigned char c:value) { out+=digits[c>>4]; out+=digits[c&15]; }
  return out;
}
}
Client::Client(std::string target,const Credentials& credentials,std::string server_name) {
  if(credentials.roots.empty() || credentials.certificate.empty() || credentials.private_key.empty()) throw std::invalid_argument("mTLS material required");
  grpc::SslCredentialsOptions tls;
  tls.pem_root_certs=credentials.roots; tls.pem_cert_chain=credentials.certificate; tls.pem_private_key=credentials.private_key;
  grpc::ChannelArguments args;
  args.SetInt("grpc.enable_retries",0);
  args.SetMaxReceiveMessageSize(1'048'576); args.SetMaxSendMessageSize(1'048'576);
  if(!server_name.empty()) args.SetSslTargetNameOverride(server_name);
  channel_=grpc::CreateCustomChannel(target,grpc::SslCredentials(tls),args);
}
grpc::Status Client::call(std::string_view method,const google::protobuf::Message& request,
                         google::protobuf::Message& response,std::chrono::milliseconds timeout) {
  response.Clear();
  if(method.starts_with('/')) method.remove_prefix(1);
  const auto slash=method.find('/');
  if(slash==std::string_view::npos) return {grpc::StatusCode::UNIMPLEMENTED,"a bundled unary method is required"};
  const auto* service=google::protobuf::DescriptorPool::generated_pool()->FindServiceByName(std::string(method.substr(0,slash)));
  const auto* rpc=service?service->FindMethodByName(std::string(method.substr(slash+1))):nullptr;
  if(!rpc||rpc->client_streaming()||rpc->server_streaming()) return {grpc::StatusCode::UNIMPLEMENTED,"method outside bundled unary client baseline"};
  if(request.GetDescriptor()->full_name()!=rpc->input_type()->full_name() || response.GetDescriptor()->full_name()!=rpc->output_type()->full_name())
    return {grpc::StatusCode::INVALID_ARGUMENT,"type differs from bundle method"};
  std::string bytes; const auto checked=wire::serialize(request,bytes);
  if(!checked.ok()) return wire_status(checked);
  const auto schema_checked=wire::validate(*rpc->input_type(),bytes);
  if(!schema_checked.ok()) return wire_status(schema_checked);
  grpc::Slice slice(bytes); grpc::ByteBuffer send(&slice,1),receive;
  grpc::ClientContext context; context.set_deadline(std::chrono::system_clock::now()+timeout);
  grpc::CompletionQueue queue; grpc::GenericStub stub(channel_);
  auto reader=stub.PrepareUnaryCall(&context,"/"+std::string(method),send,&queue);
  if(!reader) return {grpc::StatusCode::UNKNOWN,"RPC was not prepared; no result inferred"};
  reader->StartCall(); grpc::Status status; int completion=0;
  reader->Finish(&receive,&status,&completion); void* tag=nullptr; bool ok=false;
  const bool got=queue.Next(&tag,&ok); queue.Shutdown();
  void* drained=nullptr; bool drained_ok=false; while(queue.Next(&drained,&drained_ok)) {}
  if(!got||!ok||tag!=&completion) return {grpc::StatusCode::UNKNOWN,"RPC completion unavailable; no result inferred"};
  if(!status.ok()) return status;
  std::vector<grpc::Slice> slices; const auto dumped=receive.Dump(&slices);
  if(!dumped.ok()) return dumped;
  std::string data; for(const auto& part:slices) data.append(reinterpret_cast<const char*>(part.begin()),part.size());
  const auto parsed=wire::parse(response,data);
  return parsed.ok()?grpc::Status::OK:wire_status(parsed);
}
grpc::Status Client::open(const rx::contract::v1::PeerHello& hello,rx::contract::v1::Session& session,std::chrono::milliseconds timeout) {
  auto status=call("rx.contract.v1.SessionService/Open",hello,session,timeout);
  if(!status.ok()) return status;
  const std::set<std::string> features(session.required_features().begin(),session.required_features().end());
  if(!session.has_selected_version() || session.selected_version().major()!=1 || session.selected_version().minor()!=0 || hex(session.selected_version().schema_hash())!=build_info::base_manifest_sha256 || features!=std::set<std::string>{"rx.cell.v1","strict-wire-v1"}) {
    session.Clear(); return {grpc::StatusCode::FAILED_PRECONDITION,"unsupported runtime contract or required features"};
  }
  return grpc::Status::OK;
}
}
