#!/usr/bin/env python3
"""External installed-client example. JSON here is ProtoJSON, not RX canonical JSON.

Each input line is one explicit call; failures do not cause automatic resubmission.
Closing this process/transport does not cancel a runtime operation.
"""
import json
from pathlib import Path
import sys
import grpc
from google.protobuf import descriptor_pool, json_format, message_factory
from rxclpy import Client, WireError

config=json.loads(Path(sys.argv[1]).read_text())
def connect():
    return Client(config['target'], roots=Path(config['roots']).read_bytes(),
                  certificate=Path(config['certificate']).read_bytes(),
                  private_key=Path(config['private_key']).read_bytes(),
                  server_name=config.get('server_name'))
client=connect()
for line in sys.stdin:
    command=json.loads(line); result={'id':command['id']}
    try:
        if command.get('action')=='close':
            client.close(); result.update(ok=True,transport='CLOSED',operation_outcome='NOT_INFERRED')
            print(json.dumps(result),flush=True);break
        if command.get('action')=='reconnect':
            client.close(); client=connect(); result.update(ok=True,transport='RECREATED',authority='NOT_INFERRED')
        else:
            method=command['method']; service,name=method.strip('/').split('/')
            desc=descriptor_pool.Default().FindServiceByName(service).methods_by_name[name]
            request=message_factory.GetMessageClass(desc.input_type)()
            json_format.ParseDict(command['request'],request,ignore_unknown_fields=False)
            reply=client.open(request,timeout=command.get('timeout',10)) if method=='rx.contract.v1.SessionService/Open' else client.call(method,request,timeout=command.get('timeout',10))
            result.update(ok=True,response=json_format.MessageToDict(reply,preserving_proto_field_name=True))
    except grpc.RpcError as error: result.update(ok=False,code=error.code().name,detail=error.details())
    except WireError as error: result.update(ok=False,code=error.code,detail=str(error))
    except (KeyError,ValueError,json_format.ParseError) as error: result.update(ok=False,code='INVALID_ARGUMENT',detail=str(error))
    print(json.dumps(result),flush=True)
client.close()
