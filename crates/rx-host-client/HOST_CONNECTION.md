# Host bootstrap·lease·현재 상태 조회

2026-09-11. `ConnectedHost`와 `ConnectionService`를 추가했다. 지정한 Host와 연결하고 계약/신원/원장/source 세대를 대조한 뒤 Fence→grant→P 등록을 수행한다. 이는 qualification·Arm·생산 시작과 다르다.

## 읽기 계약

별도 optional `rx.host.read.v1` Inspect를 사용한다. frozen base/cell manifest는 변경하지 않았다. base/cell 협상과 등록 client certificate가 먼저 필요하며 request key/CAS는 받지 않는다. 응답은 SHA-256/size/schema가 결합한 canonical `rx.host-snapshot.v1` artifact다.

Host snapshot은 Host ID/boot, delivery/evidence journal ID, cell definition/envelope/environment, epoch/scopes/block IDs, resource fence 최대값, pending operation/permit ID와 요청한 native observations를 담는다. source별 generation·schema/unit·취득 시각·uncertainty·quality·evidence ID를 보존한다.

NativeAdapter의 `observe_sources(cell, ids)`는 read-only port이며 기본 구현은 지원하지 않는다. unsupported인 경우 sources_available=false와 빈 관측을 반환한다. 장비 세대를 Host boot로 대신하거나 연결 성공을 READY 값으로 바꾸지 않는다. FileDevice는 명시적 모의 ready source만 제공한다. 실물 ROS/PLC source mapping은 후속 adapter 의무다.

## 실제 peer 확인

HostClient의 pinned 연결은 configured CA/client key·서버 이름 검증에 더해 실제 leaf DER SHA-256을 비교한다. custom connector가 TLS stream을 완성한 뒤 Tonic에 전달한다. Tonic 내부의 transport URI만 HTTP로 지정하여 중복 TLS를 막고 요청 origin은 HTTPS로 유지한다. 입력 endpoint는 HTTPS만 허용하며 평문 연결을 반환하는 경로는 없다. 실제 test Host가 mTLS를 요구하는 상태에서 성공과 잘못된 pin 거부를 확인했다.

P에는 Host의 인증된 evidence producer가 먼저 등록되고 cell contract도 협상되어야 한다. read snapshot의 Host boot/evidence journal은 그 producer와 같아야 한다. 다른 원장이나 incarnation이면 attention이다. 서버가 주장한 Host ID 하나만 신뢰해 P 사용자/Host session을 만들지 않는다.

## 저장 후 전달

P PrepareHostLink는 현재 접근권·software lifecycle·cell revision·definition/envelope·source descriptor/generation·remote scope를 검사한다. Host가 P보다 높은 epoch를 기억하거나 알 수 없는 block을 갖고 있으면 진행하지 않는다. pending work가 있으면 새 lease를 만들지 않는다.

grant fence는 Host의 persisted resource maximum과 P의 이미 배정한 maximum보다 높게 영속 배정한다. 여러 cell이 같은 Host resource의 유효 grant를 독립적으로 교체하지 않게 한다. 다른 기존 registration의 incarnation/source와 충돌하거나 active run이 있는 신규 acquisition은 거부한다. 하나의 shared grant를 여러 cell에 묶는 전체 조정은 아직 제공하지 않는다.

Plan에는 두 요청 ID, platform-side Host session, Host incarnation·원장·source generation, 대상 scope, resource fence, TTL과 준비 시각을 저장한다. 같은 현재 plan은 재사용한다. 동일 Host boot에서 delivery/evidence journal ID가 바뀌면 새 전송 세션에서도 재사용·재획득을 거부하며, 원래 plan/request ID를 보존한다. 준비/확정은 100 ms의 현재 snapshot window에 묶인다.

네트워크에서는 계획한 Fence와 AcquireGrant를 보내고 응답이 같은 Host boot/journal/target인지 확인한다. grant의 P expiry는 **계획 시점의 영속 lower bound + TTL**이다. 같은 grant 요청을 다시 보낼 때 나중의 송신 시각으로 expiry를 늘리지 않는다.

CommitHostLink는 P의 최신 cell/producer/배정 fence와 응답을 다시 검사하고 HostRegistration·Fence 근거·원래 commit receipt를 원자적으로 저장한다. 같은 요청의 재전송은 원래 결과이며 다른 응답 body는 conflict다. 확정 전에 P 상태가 바뀌면 거부한다. unused grant가 있었다는 사실로 native 작업을 만들지 않는다.

## 갱신과 서비스 수명주기

lease 갱신도 request ID/sequence·원래 송신 lower bound를 먼저 저장한다. 응답을 잃으면 같은 pending renewal을 재사용한다. Host boot·grant ID·owner/resources/fence/TTL과 현재 cell scope가 달라졌으면 기존 authority를 새로 채택하지 않는다. 늦은 응답을 근거로 유효기간을 재시도 시각부터 계산하지 않는다.

ConnectionService는 등록된 publisher를 기다리고 최대 2초까지 backoff하며 초기 연결을 시도한다. 연결 후 기존 Dispatcher를 붙이고 lease 만료 전에 갱신한다. 실패하면 내부 상태 구독에 Attention을 발행하되 fencing/evidence 처리를 위한 sender는 유지한다. 새로운 Host boot나 source generation을 자동으로 다시 승인하거나 native operation을 재제출하지 않는다. owner가 닫히면 sender의 현재 bounded pass를 마친 뒤 종료한다.

## 플랫폼 실행 설정

`rx-platformd` startup config의 optional `host_links` 항목:

| 필드 | 의미 |
|---|---|
| host, cell | 현재 P에 등록·허용된 Host/cell |
| uri, server_name, server_fingerprint | TLS endpoint·검증할 DNS/IP 이름·정확한 leaf digest |
| tls.certificate/key/ca | 기존 pinned-file 형식. P client certificate/key와 Host server CA |
| ttl_ms | 1000–30000 ms의 lease |

중복 host/cell 항목과 범위를 벗어난 TTL은 거부한다. key 파일은 기존 private file 검사를 사용한다. Host link가 아직 없어도 P의 API/진단은 기동한다. 이 항목이 있다고 driver·Host process를 만들거나 qualification을 부여하지 않는다. S runtime draft의 기본 entrypoint도 여전히 장비를 시작하지 않는다.

## 아직 분리된 부분

- snapshot의 지속적·원자적 observation ingest와 독립 유지 조건 만료 감시를 연결했다. 상세는 [관측 수집](../rx-application/OBSERVATION_INGESTION.md)을 따른다. 연결 등록 자체는 P 조건을 PASS로 만들지 않으며, 실제 관측을 현재 source/구성·age 기준으로 별도 판정한다.
- P/Host 재부팅으로 바뀐 incarnation의 operating registration 재승격은 명시적 recovery/rebind 절차가 필요하다. 실행 권한의 재승격은 현재 거부하고 원래 작업/lease를 보존한다. 같은 인증 Host의 source 세대 변화 관측은 raw 근거와 철회로 기록하며 새 세대를 운전용으로 승인하지 않는다.
- Host 연결/조회/전달 상태 구독은 [진단 registry와 운영 화면](../rx-runtime/HOST_SERVICE_HEALTH.md)에 연결했다. 로컬 플랫폼 상태 파일의 전체 서비스 요약은 후속이다.
- 실제 ROS/native source reader, 자동 Host 프로세스/driver activation, shared-resource multi-cell lease 조정과 전체 supervision·qualification은 미완료다.

## 검증 범위

core 시험은 준비/commit rollback·응답 유실·요청 충돌, boot/journal/source/pending/epoch 거부와 갱신 sequence/시간을 확인한다. 별도 Host process를 사용하는 실제 TLS 통합에서는 pinned connection과 잘못된 pin 거부, bootstrap read, 자동 초기 등록, 같은 lease 회수/갱신을 기존 native 제출·응답 유실·두 소재 시도 경로와 함께 확인한다.

이 통합의 incoming producer 등록은 검증된 credential adapter 경계의 명시적 fixture다. 실제 Host→P publisher 협상은 기존 별도 TLS 시험으로 검증한다. 두 방향을 한 제품 launcher가 생성하는 전체 자동 구성/실물 시험으로 확대하지 않는다.
