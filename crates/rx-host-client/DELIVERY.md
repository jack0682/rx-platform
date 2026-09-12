# 자동 전달과 수신 기록 조정

`delivery::Dispatcher`는 인증/협상을 완료한 Host마다 하나씩 구성하는 P의 sender/reconciler다. application은 기존 단일 writer에서만 변경한다. 장비 SDK를 직접 호출하거나 BT의 성공값으로 결과를 결정하지 않는다.

## 처리 경계

1. SQLite pending outbox를 exclusive key 페이지로 읽는다. 앞쪽 미결 항목이 뒤쪽 항목을 영구적으로 가리지 않도록 cursor를 순환한다.
2. `plan_delivery`가 현재 Host 권한과 메시지 관계를 확인한다. 최초 전달은 같은 transaction에서 current run/permit/조건을 재검사하고 NEW→EMIT_ENTERED로 바꾼다.
3. Arm/Prepare/Authorize/Fence를 실제 mTLS HostClient로 보낸다. 이미 EMIT_ENTERED였던 operation 메시지는 GetReceipt로 조회한다. SEND_ENTERED 이상의 native 호출을 재전송하지 않는다.
4. receipt와 해당 outbox 완료를 application transaction으로 기록한다. PREPARED는 준비 전달의 ack이며 Authorize 전달 완료의 ack은 아니다.
5. native 진입이 알려진 작업은 GetReceipt→Reconcile→같은 T2로 조정한다. 결과와 자원 인계는 별개다. 이 dispatcher가 resource release나 part 완료를 자동 생성하지 않는다.

이미 저장한 동일 receipt를 다른 관련 delivery가 다시 회수할 때도 그 delivery는 완료된다. 잘못된 message/operation/Host 관계를 cached receipt 때문에 건너뛰지 않는다.

## 불명·재시도

- timeout/미수신/NOT_FOUND는 NOT_EXECUTED의 근거가 아니다. EMIT_ENTERED operation은 UNKNOWN/격리와 영속 attention을 남긴다.
- 이전 authorization의 조회 결과가 PREPARED이면 REAUTHORIZATION_REQUIRED를 남긴다. 이전 grant/permit로 재전송하지 않는다. 새 grant·현 상태 확인·permit 재결합의 복구 절차는 후속이다.
- Host session/권한이 바뀌면 stale 메시지를 새로운 권한으로 재작성하지 않는다. Host 재등록은 composition/복구 관리자의 역할이다.
- evidence가 invocation receipt보다 먼저 도착하면 원본을 보존한다. receipt가 상관관계를 확정할 때 같은 transaction에서 재평가한다. 이미 native entry를 입증한 경우 permit를 소비하고 아직 NEW인 authorization을 폐기한다.
- 결과의 불명은 poll 횟수로 해소하지 않는다. completion policy와 실제 근거만 결과를 바꾼다.

## 운용 한도

한 pass에 delivery8개/reconciliation8개, 페이지 안에서는 Fence 우선이다. HostClient RPC 제한은3초다. 다른 Host는 별도 loop로 구성한다. 재시도 backoff는100ms→최대5초, scheduling metadata는 최대1,024개다. 미결 outbox는 메모리 한도와 관계없이 DB에 남는다.

종료는 현재 제한된 pass를 마친 뒤 수행한다. 이미 claim한 send를 CAS와 RPC 사이에서 취소하지 않는다. 이 한도는 현장 보호 반응 시간의 보장이 아니다. 물리 보호는 독립된 현지 경로가 담당하며 제품 supervisor의 admission 중단·지지 확인·종료 정책 연결은 후속이다.

## 현재 제한

- Host link는 미리 인증·등록되어 있어야 한다. 자동 발견·새 grant·rebind·restart 전체 재조정은 아직 아니다.
- Reconcile은 Host journal의 초기128개 prefix다. 큰 journal의 전체 전달에는 별도 `publication::Publisher`가 필요하다.
- reconciliation 대상과 초기 상관관계 재평가는 현재 entity scan을 사용한다. 대규모 index/실측은 미완료다.
- attention은 영속 진단이며 복구 case/절차 UI 전체와 아직 결합하지 않았다.
- 제품 image/process 구성, 공개 원장 wire mapping, 시각 편집/BT, native cancel/stream, 전체 복구·인수는 계속 구현한다.

## 검증

`tools/test_host_e2e.sh`의 자동 경로는 실제 dedicated writer와 별도 Host 프로세스를 사용한다. 첫 Authorize가 모의 호출을 마친 직후 응답을 잃게 한다. dispatcher는 GetReceipt/Reconcile로 회수하며 두 part의 Authorize2회·독립 device effect2회를 확인한다. 이후 Hold의 Fence가 Host에 자동 적용되는 것도 확인한다.

part/activation 생성, 인계 proof 취득·release, part 완료는 시험 executor가 명시적으로 수행한다. 완전한 자율 생산 서비스나 실제 로봇 인수로 표현하지 않는다.
