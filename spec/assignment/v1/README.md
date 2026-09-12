# Executor assignment discovery v1

별도 선택 binding `rx.executor.assignment.v1`은 현재 Executor가 담당 셀의 시작 관계를 조회한다. 기존 base/cell 규범과 production v1 hash를 변경하지 않는다. `InspectCell`은 현재 mTLS/session/셀 협상/executor 배정을 검사하며 request key와 expected revision이 없는 읽기만 허용한다.

같은 control cut에서 installation/store/runtime·caller session·cell revision/epoch/scope·현재 definition·P 시각과 최대100ms 유효기간을 반환한다. `NONE`은 후보0, `SINGLE`은 후보1의 완전한 결과, `AMBIGUOUS`는 서로 다른 두 후보의 증거다. 두 후보는 우선순위나 선택 결과가 아니다. 현재 저장 adapter는 scan을 materialize한다. 10,000개 초과 Run 행에서는 조회를 거부하며 NONE으로 대체하지 않는다. 이 제한은 저장 allocation의 상한을 보증하지 않는다. wire payload 상한은65,536bytes다.

해당 셀의 EXECUTING/PAUSED/RECOVERY_REQUIRED와 pending StartAttempt가 있는 PREPARED가 후보다. 아직 시작을 요청하지 않은 PREPARED와 COMPLETED/ABANDONED는 제외한다. 만료된 attempt·옛 executor session·과거 구성을 조회에서 숨기지 않는다. pending attempt는 정확한 Run/Cell 관계를 검사하고 과거 configuration은 Run 소유 기록으로 읽는다. 훼손된 관계는 조회 실패이며 후보 없음이 아니다. 해당셀 후보가 두 개면 다른 후보를 열거하지 않는다.

조회는 Run/StartAttempt/mandate/원장을 생성하지 않는다. admission_allowed나 시작 허가는 반환하지 않는다. 미연결 직렬 실행기는 SINGLE의 현재 실행 권한과 실제 공정/원장을 다음 기존 Production.Inspect로 재검증해야 한다. ARMING/PAUSED/RECOVERY_REQUIRED/옛 session에서 자동 시작하지 않는다. 이미 A에 연결된 실행기는 독립 B가 생겼다는 이유만으로 A를 철회하지 않는다. 기존 scope/resource 동시성과 P의 최종 admission 검사를 유지한다.

DTO와 binding manifest를 별도 고정하며 상태/cardinality/중복 witness/시각/cut을 양쪽에서 검증한다. 같은 셀에 대한 전역 단일 Run 또는 스케줄러 정책을 추가하는 계약이 아니다.
