# 관측 묶음 수집·유지 조건 만료 감시

2026-09-11. Host 관측을 P의 조건 근거로 저장하는 경로와, 새 관측이 없을 때 유효시간을 확인하는 경로를 연결했다. 관측은 qualification·grant·Arm·작업 완료를 대신하지 않는다.

## 책임과 흐름

| 위치 | 책임 |
|---|---|
| S NativeAdapter / Host read | 실제 source의 세대·값·시각·품질·evidence ID를 반환. 지원하지 않는 source를 READY로 채우지 않음 |
| P HostClient / ObservationReader | 인증된 Host 조회를 반복하고 정확한 snapshot을 내부 writer에 전달 |
| P application | 현재 producer·Host·구성 대조, 전체 source 검증, 원자적 기록, 유지 조건 판정 및 철회 |
| P 독립 만료 감시 | 네트워크 조회와 별개로 오래된 관측에 의존한 유지 조건을 재평가 |
| P 운영 권한 처리 | 별도 qualification·시작 절차·grant·permit를 계속 요구 |

`rx.host.read.v1`의 기존 optional binding을 그대로 사용한다. frozen base/cell 계약과 공유 SDK는 변경하지 않았다. 새 `HostRead`와 `BatchReceipt`는 내부 application 타입이며 외부 사용자가 임의의 승인 상태를 전달하는 API가 아니다.

## 묶음을 받는 조건

1. 현재 Host 역할과 인증 세션이 유효해야 한다. 연결 계획이 확정돼 있고 현재 evidence producer와 boot·두 journal·session이 일치해야 한다.
2. 현재 셀 definition/envelope/environment 및 협상된 정의와 같아야 한다. Host가 P보다 높은 epoch/scope를 주장하면 거부한다.
3. 해당 Host에 지정된 source를 정확히 포함해야 한다. 중복 source/evidence ID·누락·schema/unit 불일치·허용 범위를 벗어난 uncertainty는 묶음 전체를 쓰기 전에 거부한다.
4. read 시작·snapshot capture·P 수신의 시계와 순서를 검사하고 source 취득 시각이 capture보다 미래이면 거부한다. 초기 grant 준비의 100 ms 창을 관측 기록에 적용하지 않는다. 지연된 관측도 원래 취득 시각으로 저장하며, 실제 age가 초과된 값은 조건 평가에서 UNKNOWN이다. 더 최신인 현재 기록을 과거 값으로 덮어쓰지 않는다.
5. source maximum age는 P의 FactSpec에서 가져온다. Host가 임의의 더 긴 유효시간을 선언하지 못한다.

관측을 받기 위해 실행 grant가 아직 유효하거나 P와 H의 epoch가 완전히 같을 필요는 없다. 같은 장비·구성의 관측은 정지·철회 이후에도 조사에 필요하다. 따라서 뒤처진 H epoch는 받아들일 수 있지만, 이를 이용해 P의 epoch·차단·qualification·mandate를 복원하지 않는다. 장비 세대 변경은 정상 관측으로 채택하지 않는다.

## 저장과 판정의 원자성

`report_facts`는 한 셀의 최대 128개 source를 한 transaction에서 처리한다. 모든 descriptor를 먼저 검증하고, immutable evidence와 각 source의 현재 값을 저장한 뒤 유지 조건을 평가한다. 여러 source를 하나씩 갱신하는 중간 상태를 조건 평가에 사용하지 않는다.

시험의 예는 `A 또는 B` 유지 조건이다. 기존 A=true/B=false가 A=false/B=true로 바뀔 때 묶음 전체의 조건은 계속 참이다. 첫 값만 바뀐 순간을 보고 허가를 철회하지 않는다. 이는 P 저장과 판정의 일관성에 관한 보장이다. 서로 다른 센서를 실제로 동시에 측정했다는 보장은 아니며, 필요한 물리적 측정 일관성은 장비 profile과 adapter가 별도로 입증해야 한다.

| entry disposition | 의미 |
|---|---|
| CURRENT | 현재 source 기록으로 저장 |
| DUPLICATE | 같은 immutable evidence가 이미 현재 값. 같은 상태 전이를 재실행하지 않음 |
| HISTORICAL | 현재 기록보다 이전 취득 시각. 과거 근거를 보존하고 현재 값은 유지 |
| GENERATION_CHANGED | 선언된 source 세대와 다름. raw 근거를 보존하고 현재 quality를 사용할 수 없게 하며 영향 범위를 철회 |
| INTEGRITY_CONFLICT | 같은 evidence ID의 다른 내용. 원래 근거와 새 모순을 보존하고 영향 범위를 철회 |

무결성 모순이나 세대 변화는 내용이 해석 가능한 보고다. 해당 사건·차단과 같은 묶음의 다른 유효 관측을 함께 commit하고 결과에 구별해 돌려준다. 반면 잘못된 descriptor나 다른 Host/구성으로 온 묶음은 어떤 현재 값도 갱신하지 않는다. 저장 장애는 묶음 전체를 rollback한다. commit 후 응답을 잃어 다시 보내도 원래 evidence ID를 바꾸지 않는다. 재수집 결과의 disposition은 현재 저장 상태를 보고하므로 제어 명령의 고정 영수증과는 다르다.

새 source 세대를 반복 관측해도 같은 세대 상실을 매번 새 철회로 만들지 않는다. disputed 상태도 계속 유지되는 동안 차단을 증식시키지 않는다. 새 정상 관측 뒤 다시 disputed로 바뀌면 새 이상으로 기록한다. 정상 관측은 기존 차단이나 철회된 mandate를 해제하지 않는다.

## 지속 조회와 만료 감시

연결 서비스는 dispatcher·lease 갱신과 함께 ObservationReader를 소유한다. 조회 간격은 해당 Host source의 최소 maximum age의 1/3을 기준으로 1–250 ms 범위에서 정한다. 놓친 tick을 한꺼번에 재생하지 않으며 한 reader는 한 요청씩 처리한다. 조회 실패·미지원 응답은 내부 `Unavailable` 상태로 표시하고 거짓 음성값이나 새 시각을 만들어내지 않는다. `Received`는 기록 결과를 받았다는 뜻이며 source의 유효함이나 조건 PASS를 뜻하지 않는다. 주기 조회는 과거의 모든 변화나 순간 신호를 포착하는 event journal이 아니며, 그런 근거는 장비의 별도 latched 상태/이벤트 계약이 필요하다.

별도 P 감시는 25 ms 주기로 현재 활성 Run/시작 시도의 유지 조건을 검사한다. reader가 네트워크 응답을 기다리거나 실패해도 이 감시는 계속된다. 만료·미확정·실패로 유지 조건이 상실되면 같은 transaction에서 영향 범위의 mandate/permit를 철회하고 Fence 전달을 준비한다. 원래 관측의 값·시각·품질은 고치지 않는다. 실제로 오래된 true는 오래된 true로 남는다.

25 ms는 현재 스케줄링 설정이다. 전체 탐색·writer 대기·OS 스케줄링을 포함한 최악 반응시간을 검증한 수치가 아니며 실시간 보호 기능을 뜻하지 않는다. 짧은 source TTL을 가진 실제 profile은 취득·전달·판정 지연을 포함한 별도 성능 검증이 필요하다.

P 실행 파일은 Host 연결 설정이 없더라도 이 감시를 하나만 기동한다. writer 오류를 숨기지 않고 서비스 실패로 올린다. 종료 시 reader와 dispatcher를 함께 거두며, 중단된 조회로 native 작업이나 재시작을 만들지 않는다.

## 검증과 남은 범위

- 두 source의 전체 판정, 잘못된 마지막 source의 무기록 거부, commit 전/후 장애와 재전송을 시험한다.
- 모순과 다른 source의 사실을 함께 보존하는지, 같은 세대 상실·지속 disputed의 차단이 증식하지 않는지 시험한다.
- 유효시간 경계와 만료 감시 rollback·한 번 철회·새 값 수신 후에도 이전 mandate 미복원을 시험한다.
- 실제 별도 TLS Host에서 `ConnectedHost.observe`와 반복 reader를 사용한다. 자동 dispatcher·응답 유실·두 소재 시험을 계속 통과해야 한다. 수집 반복이 native 실행 횟수를 늘리지 않는지도 같은 시험에서 확인한다.

실물 source mapping·profile qualification, 운영 화면의 관측/연결 상태 통합, 명시적 rebind, 현장 반응시간과 기록 보존 정책은 미완료다. 현재 immutable evidence와 사건 기록은 수집량에 따라 증가한다. 장기간 운영의 용량·보존·반출 정책을 아직 제공하지 않으며, 현장 상시 운영 지원으로 표시하지 않는다. 첫 물리 셀은 NOT_COMMISSIONED다.
