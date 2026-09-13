# 정상 Host binding의 불변 근거

`PrepareHostLink.provenance`와 저장 Plan의 같은 필드는 optional이다. 실제 pinned transport와 Host configuration/read 검증을 마친 내부 HostClient만 `Some(BootstrapProvenance)`를 제공한다. 기존 legacy `None`은 계속 읽을 수 있으며 직렬화에서 필드를 생략한다. legacy plan을 조회하거나 commit 재전송할 때 근거를 소급 생성하지 않는다. 반대로 이미 provenance가 있는 plan을 `None`으로 낮추는 요청은 만료 후에도 거부한다.

TransportPin은 public HTTPS origin·server name, 사용한 CA/client chain PEM bytes의 SHA-256, 실제 Host/P client leaf DER SHA-256, release와 실제 base/cell/HostRead/HostConfig wire hash를 고정한다. URI에는 사용자 정보·query·fragment·임의 path를 허용하지 않는다. private key, Runtime instance, lease TTL은 pin에 포함하지 않는다. TransportPin digest는 이 공개 필드의 canonical domain digest다.

BootstrapProvenance는 실제 Host configuration Observation과 그 P read window, 기존 Prepare.snapshot과 exact canonical 일치하는 HostRead 원문을 보존한다. configuration read는 같은 shared clock의 최대100ms 구간이어야 하며 HostRead보다 먼저 끝나야 한다. Host configuration과 HostRead의 host/cell/boot/delivery journal/definition/envelope/environment/epoch/scopes/blocks를 대조한다. P에 대해서는 기존 Host epoch/scopes가 P 이하인 bootstrap 규칙을 유지한다. 뒤따르는 원래 Fence ACK가 정확한 P bound cut을 확정한다. 초기 applied context None은 유효한 관측이며 apply 완료를 의미하지 않는다.

새 정상 CommitHostLink가 실제 registration·fence receipt·BoundReceipt·bound Plan을 저장하는 **같은 transaction**에서 plan ID를 key로 한 `HostBindingBaseline`과 원문 sidecar를 각각 revision1로 한 번 생성한다. commit rollback에는 둘 다 없고 응답 유실 뒤 exact replay에는 원래 값만 남는다. 다른 commit body, 재사용 plan의 바뀐 transport/configuration identity, 이미 존재하는 baseline/source row는 충돌이다. Bound로 이미 확정된 plan의 replay는 baseline 생성 코드를 다시 실행하지 않는다.

baseline은 installation/store/runtime, producer authentication/session, 양방향 session, Host boot·양쪽 journal·source map, transport와 digest, 원래 P configuration digest, Host binding digest와 applied context, 원래 bound epoch/scopes와 관측 reference를 담는다. 원문 sidecar에는 초기 bound Plan과 provenance의 HostRead/configuration/source 원문, 당시 installation/producer/P CellConfiguration, immutable BoundReceipt를 보존한다. baseline은 sidecar **전체 canonical 원문**의 ArtifactRef도 고정하므로 별도 projection에 없는 producer.cells·installation.clock_id도 hash/size 검증에 포함된다. sidecar는 baseline을 포함하지 않아 참조 순환이 없다. baseline read는 원문의 hash·size·schema·상관과 original plan/registration digest를 다시 검사하고 저장 BoundReceipt와도 비교한다. 원문 누락·변조·잘못된 reference를 legacy None이나 현재 근거로 대체하지 않는다.

primary baseline이 없더라도 source가 남았거나 known bound Plan이 provenance Some이면 무결성 오류다. Plan 없이 BoundReceipt만 남은 모순도 초기 부재로 보지 않는다. 실제 legacy None과 아직 bound하지 않은 plan, 아무 이력도 없는 plan ID의 조회만 None을 유지한다. 누락된 불변 기록을 조회나 replay에서 다시 만들지 않는다.

`Engine::host_binding_baseline(plan)`은 내부 historical read다. 현재 qualification·grant·Arm·시작 권한을 반환하지 않는다. 원래 plan/registration digest는 역사적 객체를 식별하며, 갱신된 현재 Plan.valid_until이나 정상 변경 이후의 전체 HostRegistration/P configuration과 비교하지 않는다. 별도 `stable_identity()`는 installation/store와 Host/cell/Host boot/journals/source map/transport digest만 투영한다. Runtime/session/lease·현재 epoch/구성을 이 projection으로 자동 복구하지 않는다.

이 단계는 RecoveryBinding, 명시 rebind HTTP/CLI, qualification/Arm 복구를 구현하지 않는다. 기존 prepare/commit/renew의 권한·세대·lease 검사를 완화하지 않으며 base/cell 규범이나 SDK/공개 wire를 변경하지 않는다.

전용 시험 소스는 최초 None applied context, P보다 낮은 pre-Fence H cut, 원문·참조 검증, commit rollback/응답 유실/replay, legacy no-backfill, pin 변경·None downgrade, configuration/window 반례, 갱신·후속 epoch 변경 후 역사 읽기를 다룬다. 실제 pinned transport/optional configuration read와 제품 통합은 별도 HostClient 시험에서 검증한다.
