# RX client libraries — G5.1

Two language interfaces consume one checked protocol bundle. They perform strict
wire validation and mTLS unary calls; they do not own devices, issue operating
permission, store authoritative results, automatically resubmit or cancel work.

This slice has **no connected ROBOTIS robot/adapter**. It does not yet establish
that the common contract carries a ROBOTIS product, and completes neither the G5
SDK baseline nor the five-product development bundle. The next G5.2 slice connects
one Host-owned, simulated read-only DYNAMIXEL Ping helper; real device endpoints
and torque/motion remain excluded.

## Prepare, build and install

Use Python>=3.10, a matching C++ Protobuf compiler/runtime, CMake>=3.22, gRPC C++,
C++20, OpenSSL and nlohmann_json (the last two for conformance/examples). The tested
Linux image uses Ubuntu24.04/arm64, protoc/libprotobuf3.21.12, gRPC C++1.51.1 and
Python3.12; Python dependencies are pinned in rxclpy/pyproject.toml. CMake consumers
must use the exact build's Protobuf version because its C++ ABI is not portable
across releases. No ROS dependency is required for these client interfaces.

```sh
python3 tools/export_protocol.py /absolute/new-bundle
python3 tools/prepare_clients.py --bundle /absolute/new-bundle --output /absolute/new-clients
cmake -S /absolute/new-clients/rxclcpp -B /absolute/cpp-build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/absolute/install
cmake --build /absolute/cpp-build -j2
cmake --install /absolute/cpp-build
python3 -m venv /absolute/client-env
/absolute/client-env/bin/pip install build==1.6.1 setuptools==84.0.0 wheel==0.48.0
/absolute/client-env/bin/python -m build --wheel --no-isolation /absolute/new-clients/rxclpy
/absolute/client-env/bin/pip install /absolute/new-clients/rxclpy/dist/rxclpy-0.1.0-py3-none-any.whl
/absolute/client-env/bin/python -m rxclpy.conformance
cmake -S clients/examples/cpp -B /absolute/external-build -DCMAKE_PREFIX_PATH=/absolute/install
cmake --build /absolute/external-build -j2
```

The generated packages can be copied to a separate developer checkout; consumers
need neither Rust source includes nor a new core build. Preparation refuses missing
profile files, altered bundled bytes and a preexisting output. A raw, unprepared
client source tree is not an installable generated package. Bundle digests establish
consistency, not publisher trust or runtime authority.

`tools/test_client_packages.py --work /absolute/fresh-work` runs both installed
consumers and the same authored corpus. Run it inside a disposable Python venv
containing the pinned dependencies; it installs the built wheel into that venv.
CMake `rxclcpp::rxclcpp` is the installed target. Python generated modules live under
`rxclpy._generated`, avoiding an unrelated top-level `rx` Python package collision.

## Runtime calls and failures

Use `Client.open` with a caller-owned PeerHello, then `Cell.Open` and the existing
`Cell.SubmitOperation`/`Operation.Get` path. The base Operation.Submit and Lookup are
intentionally unimplemented: they must not bypass cell admission. To recover a
lost admission reply, explicitly retry the same cell request/key; never invent a
new operation because a transport call timed out.

`examples/python_peer.py` and the external CMake `rxclcpp-peer` read a TLS config
and explicit JSON-line commands. Their JSON is **ProtoJSON**, not RX canonical JSON
for domain digests/signatures. A successful process exit or Session.Open is not
operational readiness. A successful RPC receipt is admission, not native completion.
Read the actual OperationView knowledge/outcome. Both examples preserve remote
status codes; Python raises grpc.RpcError, while C++ returns grpc::Status. Local
wire failures are separately categorized INVALID_ARGUMENT/RESOURCE_EXHAUSTED.

No automatic transport retry is enabled. Closing a client closes transport only;
there is no implicit Operation.Cancel, registration deletion or outcome inference.
A new client incarnation must use its own boot identity and renegotiate. That does
not restore an old mandate or permit. G3/G4 supervisor work-use has no gRPC ingress
and is not proxied, judged or synthesized here; platform cell authorization remains
owned by P. TLS client credentials are not operating-area decision-signing keys.

`tools/test_clients.py` runs actual current P/H binaries, existing signed-package
compilation and public terminal approval/qualification, and FILE_SIMULATION. It
substitutes the installed SDK only for E's client role. A keyless TCP test tunnel
can drop P-to-client TLS records after negotiation; native file effects and P
records remain independent observations. Readable but unwritable effect storage
exercises true native I/O uncertainty. No direct database authority seed is used.
Unknown cases do not claim clean whole-cell shutdown; test cleanup is separate.

The normative profile is [strict-wire-v1](../proto/strict-wire-v1/README.md). The
82 rule-authored cases include69 public-message and13 TCK-only descriptor checks.
No encoder produces the expected vectors. Private-source duplicate-singular and
enum-zero mutants demonstrate two independently detected rule failures. Parsing
work limits can differ for equivalent packed/unpacked representations, so exact
byte roundtrips and universal decode-then-reencode closure are not promised.
