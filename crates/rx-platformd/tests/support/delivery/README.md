# 실제 두 이미지 SIMULATION 인수 입력 exporter

모든 진입점은 `rx-platformd`의 ignored test이며 Runtime/Host/Executor session·qualification·Run을 만들지 않는다. P 생산 authority는 변경하지 않는다. 외부 harness가 실제 S image의 CLI, 두 제품 이미지와 브라우저/API를 실행한다.

## A: 공정·패키지 입력

`RX_CELL_DELIVERY_OUTPUT=<새 절대 디렉터리>`와 선택 `RX_CELL_DELIVERY_ARCH=arm64|amd64`(기본 arm64)를 설정해 `cargo test -p rx-platformd --test delivery_fixture --locked export_delivery_seed -- --ignored --exact`를 실행한다.

출력은 `seed.json`, `initial-cell.json`, `compile-input.json`, `package-recipe.json`, `package-policy.json`, `assets/<sha>.bin`, `scope.json`, `signing-fixtures.json`이다. base/cell hash는 실제 checked-in protocol_manifest의 canonical bytes에서 계산한다. source는 FILE_SIMULATION을 위한 one-action 공정이며 ready 관측, sim/ready guard, rx.sim.completed.v1 code0 완료를 명시한다. program/parameter와 모든 semantic digest도 실제 입력 bytes가 있다.

S CLI에는 private signer 파일을 제외한 public seed 사본만 `/config:ro`로 제공한다. `assemble → request → 외부 test signature → seal → compile`을 실제 `rx-process-package`로 실행한다. 최종 compile은 **봉인한 package**에서 실행해야 package_digest가 실제 manifest를 참조한다. `validator-identity` stdout의 validator_digest도 보관한다. exporter는 ResolvedProcess나 compiler verification report를 조작·생성하지 않는다.

## 외부 시험 서명

`sign_delivery`에는 `RX_CELL_DELIVERY_SEED=<A>`, `RX_CELL_SIGN_INPUT=<절대 JSON>`, `RX_CELL_SIGN_OUTPUT=<새 절대 signature JSON>`을 제공한다. 입력은 아래 두 형식 중 하나다.

```json
{"key":"delivery-package-signer","message_hex":"<실제 S signing request의 message_hex>"}
```

```json
{"key":"delivery-qualification-signer","qualification_report":"<부모가 실제 증거로 작성한 qualification.json의 절대 경로>"}
```

허용 key는 delivery-package-signer, delivery-review-signer, delivery-qualification-signer다. S signing-request JSON의 다른 metadata는 입력에 복사하지 않고 key/message_hex만 전달한다. 출력 signature envelope 외에 `.metadata.json` 확장자의 sidecar가 생긴다(예: `verification.sig.json` → `verification.sig.metadata.json`). qualification 모드의 sidecar에는 정확한 Report::digest가 들어 있으며 signing_message도 기존 q 모델에서 계산한다. 정책·검증·실행 권한을 발급하지 않는다.

시험용 private seeds는 A의 `signing-fixtures.json`에만 저장한다. P/S runtime config·이미지·증거 저장소에 복사하지 않는다. 키 ID별 공개키는 A seed와 정책에 연결되며 signer가 이를 다시 대조한다. CA private key는 B에서 내보내지 않는다.

## B: 최종 기동·재검증 정책

`export_delivery_final`의 필수 환경:

- RX_CELL_DELIVERY_OUTPUT: 새 최종 디렉터리
- RX_CELL_DELIVERY_SEED: A 디렉터리
- RX_CELL_RESOLVED: 실제 S sealed-package compile의 resolved.json
- RX_CELL_PACKAGE: 실제 S sealed package 디렉터리
- RX_CELL_COMPILER_ID: 실제 S validator-identity의 64자리 digest
- RX_CELL_QUALIFICATION_VALIDATOR_ID: 부모의 실제 evidence harness/검증절차를 식별하는 digest
- RX_CELL_DELIVERY_PORT: 외부 terminal HTTPS 포트
- RX_CELL_OPERATOR_BUNDLE: 선택한 실제 S UI dist(선택). P에서는 `/operator:ro`로 제공

B는 package 서명/내용과 signed compile-input=A, 실제 resolved의 source/package/action/구조를 검사한다. 정책 asset 경로의 로컬 검사 사본만 A 경로로 바꾸며 공개 package policy bytes는 그대로 유지한다. one-action에 대해 실제 P process-change와 같은 step ID 변환으로 target을 계산한다. **P API가 반환한 Change.after.sha256와 delivery.json의 target_configuration_digest가 반드시 같아야 한다.** 다르면 live policy를 바꾸지 말고 새 폐기 가능한 설치로 다시 시작한다.

출력:

- `config/`: P startup/catalog/credentials/public policies, P server와 P→H TLS 파일, program/parameter asset
- `host-config/`: H startup template, 공통 binding 입력, Host server/publisher TLS 파일
- `executor-config/`: E cell template와 E client TLS 파일
- `browser/`: 등록 단말 cert/key와 public CA. 컨테이너 밖에서만 사용
- `import/package/`: 검증한 실제 S signed package
- `reference/`: initial/target/physical-negative cell과 실제 resolved
- `qualification-materials/`: exact v2 policy 및 원본 artifact pool. 결과 보고서나 PASS는 생성하지 않음
- `delivery.json`: 환경/식별값/target/계약/validator/후속 patch stage
- `browser-fixture.json`: 시험 로그인 정보, 단말 경로, 역할별 허용 mount 파일 목록

installer는 bootstrap용 AccountAdmin+Engineer, engineer/verifier/release/operator는 역할을 분리한다. browser terminal은 실제 leaf DER fingerprint로 등록하고, H publisher/E fingerprint만 P gRPC service 허용 목록에 넣는다. Host/Executor session은 만들지 않는다.

P→H의 Hello.peer_id는 제품 ConnectionService와 동일한 설치 UUID의 Name이다. Host allowed_platform_certificates의 값과 binding-input.platform은 모두 이 값이며 delivery.json.platform_peer에도 기록한다. TLS client 인증서의 SAN/CN label은 이 peer ID를 대신하지 않고 실제 leaf fingerprint를 허용 목록에 결합한다.

`cell/physical-unconfigured`는 main과 다른 definition/Host/Executor/resource/scope/profile/site이며 물리 endpoint·host_link·qualification profile이 없다. 모델/주소를 추정하지 않는다. Operator/Engineer는 이 셀의 CreateRun/시작 거부를 검사할 수 있고, 그 Prepared Run은 main qualification의 quiet/impact 집합과 겹치지 않는다.

## 부모의 명시적 patch 및 실제 실행 단계

P mount는 config→/config, import→/import, 별도 데이터 volume→/data다. P data는 /data/platform, runtime은 /data/runtime이며 부모가 UID10001 소유의 쓰기 가능 디렉터리를 준비한다. gRPC는 https://p:7443, Host는 https://s:7444, terminal은 https://127.0.0.1:PORT다.

먼저 실제 P init/run과 terminal login/overview에서 installation.store_generation을 읽는다. 그 값을 Host publisher.store_generation과 E expected_service.scope.store_generation에 넣는다. Host bindings는 binding-input.json의 exact cell/Intent/condition에서 부모가 S 형식으로 만들고 sha256를 고정한다. E engine pin은 실제 S image의 /opt/rx/bin/rx-bt-engine에서 얻는다. template의 PATCH 문자열은 일부러 유효 UUID/digest가 아니므로 patch 전에는 실행할 수 없다. patch 완료 뒤 각 역할 파일을 `startup.json`/`bindings.json`/`cell.json`으로 게시한 후 H/E의 init/run을 수행한다.

P config에는 H/E/browser/signing private key가 없으며 각 역할의 mount 파일 목록은 browser-fixture.json에 있다. 임의로 최종 root 전체를 한 컨테이너의 /config에 mount하지 않는다.

나중 qualification 보고서는 P Begin의 실제 Request 전체와 부모 harness의 영역별 실제 산출물을 사용한다. software/compiler, Host/equipment, Fence/cohort integration, 검증한 복구 증거, PHYSICAL/권한 거부, 분리 계정/무운전 상태를 각 criterion의 evidence_schema로 연결한다. 미수행은 NOT_RUN이며 exporter의 spec 존재나 label을 PASS 근거로 삼지 않는다. 원장·상태·원문은 API/제품 프로그램에서만 만들고, 테스트 authority나 직접 DB seed로 첫 자격을 대체하지 않는다.
