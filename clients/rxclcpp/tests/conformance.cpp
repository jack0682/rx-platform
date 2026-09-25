#include "rxclcpp/wire.hpp"
#include "strict-wire-v1/probe.pb.h"
#include <google/protobuf/dynamic_message.h>
#include <nlohmann/json.hpp>
#include <openssl/sha.h>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>
using Json = nlohmann::json;
std::string hex(const std::string& value) {
  if(value.size()%2) throw std::runtime_error("odd hex");
  std::string result;
  for(std::size_t i=0;i<value.size();i+=2) result.push_back(static_cast<char>(std::stoul(value.substr(i,2),nullptr,16)));
  return result;
}
std::string varint(std::uint64_t value) {
  std::string result;
  while(value>=128) { result.push_back(static_cast<char>((value&127)|128)); value>>=7; }
  result.push_back(static_cast<char>(value)); return result;
}
std::string delimited(std::uint64_t tag,const std::string& body) { return varint((tag<<3)|2)+varint(body.size())+body; }
std::string expand(const Json& value) {
  if(value.contains("hex")) return hex(value["hex"]);
  if(value.contains("concat")) { std::string out; for(const auto& v:value["concat"]) out+=expand(v); return out; }
  if(value.contains("repeat")) {
    const auto& v=value["repeat"]; const auto part=hex(v["hex"]); const auto n=v["count"].get<std::size_t>();
    std::string out; out.reserve(part.size()*n); for(std::size_t i=0;i<n;++i) out+=part; return out;
  }
  if(value.contains("delimited")) { const auto& v=value["delimited"]; return delimited(v["tag"],expand(v["body"])); }
  if(value.contains("nest")) {
    const auto& v=value["nest"]; auto out=expand(v["leaf"]);
    for(std::size_t i=0;i<v["levels"].get<std::size_t>();++i) out=delimited(v["tag"],out);
    return out;
  }
  throw std::runtime_error("unknown corpus recipe");
}
std::string hash(const std::string& value) {
  unsigned char bytes[SHA256_DIGEST_LENGTH]; SHA256(reinterpret_cast<const unsigned char*>(value.data()),value.size(),bytes);
  std::ostringstream out; for(auto b:bytes) out<<std::hex<<std::setw(2)<<std::setfill('0')<<static_cast<unsigned>(b); return out.str();
}
int main(int argc,char** argv) {
  if(argc!=2) return 2;
  // Force registration of TCK-only descriptors, not a runtime service.
  (void)rx::strict_test::v1::Node::descriptor();
  std::ifstream input(argv[1]); Json corpus; input>>corpus; Json rows=Json::array(); bool pass=true;
  google::protobuf::DynamicMessageFactory factory;
  for(const auto& test:corpus.at("cases")) {
    const auto* desc=google::protobuf::DescriptorPool::generated_pool()->FindMessageTypeByName(test.at("message"));
    if(!desc) throw std::runtime_error("descriptor missing: "+test.at("message").get<std::string>());
    std::unique_ptr<google::protobuf::Message> message(factory.GetPrototype(desc)->New());
    const auto data=expand(test["input"]); auto status=rxclcpp::wire::parse(*message,data);
    Json row={{"name",test["name"]},{"schema",test["schema"]},{"expected",test["status"]},{"actual",rxclcpp::wire::name(status.code)},{"bytes",data.size()},{"sha256",hash(data)}};
    pass=pass&&(row["expected"]==row["actual"]);
    if(test.contains("roundtrip_status")) {
      if(!status.ok()) throw std::runtime_error("cannot reencode rejected input");
      std::string encoded; status=rxclcpp::wire::serialize(*message,encoded);
      row["roundtrip_expected"]=test["roundtrip_status"]; row["roundtrip_actual"]=rxclcpp::wire::name(status.code);
      pass=pass&&(row["roundtrip_expected"]==row["roundtrip_actual"]);
    }
    rows.push_back(row);
  }
  std::cout<<Json{{"cases",rows}}.dump(2)<<'\n'; return pass?0:1;
}
