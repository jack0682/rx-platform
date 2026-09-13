# 제조사별 native 결과와 코어의 결론 연결

상태: phase62 구현. 기존 단일 schema `NATIVE`와 `PREDICATE`/`UNOBSERVABLE` 형식은 유지한다. 새 `NATIVE_OUTCOMES`는 설치·검토된 셀의 StepBinding에 선언하는 결과 해석 규칙이다. HTTP 요청이나 Host evidence가 실행 중 임의의 결론을 지정하는 통로가 아니다.

## 책임과 데이터

`rx-process-contract::native_outcome`에는 ROS/장비 API를 모르는 데이터 타입만 있다. `NativeOutcomeTable`은 schema, profile_digest, completion_rule, cases를 가진다. 각 case는 정확한 native status schema와 정수 code 집합을 SUCCEEDED/FAILED/CANCELED 중 하나에 대응시킨다. P application은 이 데이터를 검사하고 영속 evidence에서 결론을 도출한다. 제조사별 상태 어휘·code의 의미는 S에서 제공한다.

`CompletionRule::NativeOutcomes { table, postconditions }`를 CellConfiguration의 StepBinding에 넣는다. table의 profile_digest와 completion_rule은 같은 step Intent와 일치해야 한다. StepBinding은 기존 구성 digest/검토/변경 문맥에 포함되며, admission 때 Work에 복사되어 과거 작업의 해석 규칙으로 보존된다. 현재 구성의 새 대응표로 과거 Work를 다시 해석하지 않는다.

table은 최대16 case, case당64 code, 전체128 schema/code 쌍을 허용한다. 빈 표·빈 code 집합·중복 쌍은 거부한다. 중복 결론이 같아도 모호한 선언을 허용하지 않는다. wildcard, 범위 비교, 기본 성공, 문자열 실행·동적 함수는 없다. schema/code 쌍이 없으면 결과를 확정하지 않는다.

NOT_EXECUTED와 UNRESOLVED는 이 대응표로 만들 수 없다. native 송신 전 미실행 입증 및 불명 작업의 명시적 복구/종료는 기존 별도 절차의 책임이다. 특히 native API가 goal을 거부했다는 사실과 송신 전 tombstone은 다르다.

## 영속 적용과 상태

기존 T2의 Host 신원/셀 권한, operation/invocation/profile 상관 확인, evidence ID와 stream sequence 충돌 검사를 유지한다. 원본 evidence·연속 cursor·Work 결과·제어 사건은 기존 transaction에서 함께 기록한다. 동일 batch 재전송은 같은 결과를 회수하고 추가 결론을 만들지 않는다.

대응표가 성공을 가리켜도 postconditions가 있으면 현재 조건 근거와 continuity를 요구한다. 만료·false·unknown 조건 또는 셀의 block/epoch 불일치가 있으면 성공으로 승격하지 않는다. 유한 action 외의 Intent는 기존 규칙처럼 성공에 필요한 postconditions를 생략할 수 없다. 실패·취소 사실에는 성공 후조건을 강제하지 않는다.

어떤 terminal 결과도 자원을 자동 해제하지 않는다. 제어/소재 지지 인계는 별도의 근거와 절차를 요구한다. 같은 작업에서 나중에 다른 terminal 결과가 확인되면 최초 outcome을 유지하고 integrity=DISPUTED, disposition=QUARANTINED 및 셀 차단을 기록한다. 결과가 불명인 상태에서 반복 조회가 있었다는 사실만으로 결론을 만들지 않는다.

## ROS JTC의 S 선언

S의 `ros_jtc::Profile::outcome_table()`이 검증된 Profile digest와 완료 규칙을 결합한 표를 만든다. 코어에는 아래 schema 문자열이나 ROS 라이브러리 의존성을 넣지 않는다.

| Native capture schema | code | 결론 |
|---|---|---|
| rx.ros-jtc.succeeded.v1 | 0 | SUCCEEDED |
| rx.ros-jtc.canceled.v1 | -5, -4, -3, -2, -1, 0 | CANCELED |
| rx.ros-jtc.aborted.v1 | -5, -4, -3, -2, -1, 0 | FAILED |
| rx.ros-jtc.goal-rejected.v1 | 0 | FAILED |
| 나머지 조합 | 전체 | 결론 없음; 원본 보존 |

이는 현재 control_msgs5.9.0의 code 범위와 phase61 adapter capture 어휘에 맞춘 명시적 정책이다. code0만 보고 성공으로 분류하지 않는다. ROS success/nonzero error의 모순은 S에서 이미 capture 생성을 거부하고 분쟁을 보관한다. 다른 code를 쓰는 controller는 별도 profile/검토가 필요하며 기본 실패나 성공으로 추측하지 않는다.

ROS의 접수/취소 요청 수용은 terminal result와 구별된다. ROS goal state와 result-cache 동작은 [공식 action 설계](https://design.ros2.org/articles/actions.html), 오류 code는 [control_msgs5.9.0 원문](https://github.com/ros-controls/control_msgs/blob/5.9.0/control_msgs/action/FollowJointTrajectory.action)을 따른다. 이 두 문헌은 기계의 물리 상태나 RX의 현장 자격을 입증하지 않는다.

## 호환성과 검증 범위

기존 규범8개 및 gRPC/Protobuf wire 형식은 바꾸지 않았다. 추가한 표는 SDK의 데이터 타입과 내부 구성/Work JSON variant다. 기존 variant의 저장 형식은 그대로 읽는다. 새 variant를 기록한 DB/구성은 이전 바이너리가 읽을 수 있다고 보장하지 않으며, 실제 release 교체/rollback은 지원 조합 검사와 백업·복원 절차를 거쳐야 한다. 이 기능이 그 배포 절차까지 구현한 것은 아니다.

시험은 표의 중복/한도/unknown 필드/불허 결론, 설치 시 profile/rule 일치, schema/code별 결과, 원본 보존·동일 batch·commit 전 실패/응답 유실·후기 모순, 현재 postcondition과 Hold를 다룬다. S 시험은 실제 JTC adapter의 native capture를 생성하고 같은 공유 타입으로 표를 해석한다. P와 S 각 경계의 검증이며 실제 로봇→전체 P 서비스의 통합 인수 시험은 아니다.

JTC 장비 package authoring/factory 및 P 구성 반입을 자동 연결하는 resolver는 후속이다. 현재 생성 함수의 반환값을 검토된 구성에 연결해야 한다. 실제 장비 Authority, controller 세대별 fencing, native cancel 조정·복구, 물리 교정/지지와 현장 인수도 남아 있다. 현재 연동 경계는 [장비 Host 문서](https://github.com/jack0682/rx-solutions/tree/main/runtime/rx-host), 당시 시험 근거는 [초안 시점 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase62_checks.json)을 본다. 과거 시험을 현재 중립 구성의 검증 결과로 승격하지 않는다.
