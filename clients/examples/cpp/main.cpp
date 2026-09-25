#include <rxclcpp/client.hpp>
#include <google/protobuf/dynamic_message.h>
#include <google/protobuf/util/json_util.h>
#include <nlohmann/json.hpp>
#include <fstream>
#include <iostream>
#include <iterator>
#include <memory>
#include <stdexcept>
using Json=nlohmann::json;
std::string read(const std::string& path) {
  std::ifstream input(path); if(!input) throw std::runtime_error("cannot read input file: "+path);
  return {std::istreambuf_iterator<char>(input),{}};
}
const char* code(grpc::StatusCode value) {
  switch(value) {
#define RX_STATUS(X) case grpc::StatusCode::X:return #X
    RX_STATUS(OK); RX_STATUS(CANCELLED); RX_STATUS(UNKNOWN); RX_STATUS(INVALID_ARGUMENT);
    RX_STATUS(DEADLINE_EXCEEDED); RX_STATUS(NOT_FOUND); RX_STATUS(ALREADY_EXISTS);
    RX_STATUS(PERMISSION_DENIED); RX_STATUS(RESOURCE_EXHAUSTED); RX_STATUS(FAILED_PRECONDITION);
    RX_STATUS(ABORTED); RX_STATUS(OUT_OF_RANGE); RX_STATUS(UNIMPLEMENTED); RX_STATUS(INTERNAL);
    RX_STATUS(UNAVAILABLE); RX_STATUS(DATA_LOSS); RX_STATUS(UNAUTHENTICATED);
#undef RX_STATUS
    default:return "UNKNOWN";
  }
}
int main(int argc,char** argv) {
  if(argc!=2) return 2;
  const auto config=Json::parse(read(argv[1]));
  const auto connect=[&] { return std::make_unique<rxclcpp::Client>(config.at("target"),
      rxclcpp::Credentials{read(config.at("roots")),read(config.at("certificate")),read(config.at("private_key"))},config.value("server_name","")); };
  auto client=connect(); google::protobuf::DynamicMessageFactory factory;
  for(std::string line;std::getline(std::cin,line);) {
    const auto command=Json::parse(line); Json result={{"id",command.at("id")}};
    try {
      const auto action=command.value("action","");
      if(action=="close") {client.reset();result.update({{"ok",true},{"transport","CLOSED"},{"operation_outcome","NOT_INFERRED"}});std::cout<<result.dump()<<std::endl;break;}
      if(action=="reconnect") {client.reset();client=connect();result.update({{"ok",true},{"transport","RECREATED"},{"authority","NOT_INFERRED"}});}
      else {
        const auto method=command.at("method").get<std::string>(); const auto slash=method.find('/');
        const auto* service=google::protobuf::DescriptorPool::generated_pool()->FindServiceByName(method.substr(0,slash));
        const auto* rpc=service?service->FindMethodByName(method.substr(slash+1)):nullptr;
        if(!rpc) throw std::runtime_error("unknown bundled method");
        std::unique_ptr<google::protobuf::Message> request(factory.GetPrototype(rpc->input_type())->New()),reply(factory.GetPrototype(rpc->output_type())->New());
        const auto parsed=google::protobuf::util::JsonStringToMessage(command.at("request").dump(),request.get());
        if(!parsed.ok()) throw std::runtime_error(parsed.ToString());
        const auto timeout=std::chrono::milliseconds(static_cast<long long>(command.value("timeout",10.0)*1000));
        grpc::Status status;
        if(method=="rx.contract.v1.SessionService/Open") {
          rx::contract::v1::PeerHello hello; hello.ParseFromString(request->SerializeAsString());
          rx::contract::v1::Session session; status=client->open(hello,session,timeout);
          if(status.ok()) reply->ParseFromString(session.SerializeAsString());
        } else status=client->call(method,*request,*reply,timeout);
        if(status.ok()) {
          std::string output; google::protobuf::util::JsonPrintOptions options; options.preserve_proto_field_names=true;
          const auto converted=google::protobuf::util::MessageToJsonString(*reply,&output,options);
          if(!converted.ok()) throw std::runtime_error(converted.ToString());
          result.update({{"ok",true},{"response",Json::parse(output)}});
        } else result.update({{"ok",false},{"code",code(status.error_code())},{"detail",status.error_message()}});
      }
    } catch(const std::exception& error) {result.update({{"ok",false},{"code","INVALID_ARGUMENT"},{"detail",error.what()}});}
    std::cout<<result.dump()<<std::endl;
  }
}
