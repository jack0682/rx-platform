# 장비 binding과 실행 허용 조건

규범: RX 계약 v1.0 · 입력: [장비 지원 정책](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md), [이미지 명세](https://github.com/jack0682/rx_docs/blob/main/docs/14_image_support_spec.md)

## 1. Profile의 역할

공통 계약은 완료/불명/권한의 의미를 정의한다. BindingProfile은 그 의미를 **특정 모델·모드·controller·펌웨어·교정·native API**에 연결한다. 제품군 이름 하나로 서로 다른 controller 구성을 합치지 않는다.

Profile은 불변 artifact이며 내용 digest로 참조한다. 필수 항목이 없는 profile은 해당 profile의 검증 backlog에 남으며 운영 admission에는 사용할 수 없다. 모델명만 맞고 controller/firmware/revision이 다르면 별도 profile이다.

| 필수 항목 | 구체 내용 |
|---|---|
| identity | support_id, manufacturer/model/hardware_revision, firmware 범위, driver/controller 버전과 source commit |
| native interface | endpoint 역할, native API/action/message type, 단위·관절/채널 순서, 반환값 의미, native ID/결과 조회 가능성 |
| dependencies | 필수 패키지·전이 의존성·이미지 variant·kernel/device/GPU 제약 |
| capability | 제공 kind/body, 관측 schema, 완료/실패/취소 근거, 현재 상태와 과거 결과 조회의 구별 |
| resource_set | controller·mode·버스·공유 gripper·공간 등 충돌 단위 |
| startup/shutdown | prepare/activate/destructor가 토크·위치·모드에 주는 효과, 실제 init flags |
| calibration | base/tool/지그/그리퍼/영점 교정 ID·범위·유효 조건 |
| mode/authority | 필요한 mode, remote/auto/수동 전환 확인, 직접 native client 배타 조건 |
| completion/cancel | predicate, correlation, 허용 오차, settling, timeout, 중단 결과, release 조건 |
| timing | observation age 근거, grant TTL, ticket/deadman, prepare validity, execution, shutdown 값과 근거 |
| restart | boot 검출, native 이력 보존, 잔류 명령, 재동기화, unobservable reboot 제한 |
| evidence | 필수 evidence schema, 인간 확인 허용 범위, 모순 판정 |
| validation | 문헌 확인/실험실/현장 결과, 실제 시험 artifact, 날짜·버전·조건·제약 |

시간/속도/토크 값은 코드의 default를 그대로 안전 기준으로 승격하지 않는다. source에 있는 update rate도 실시간 성능 측정값이 아니다. CPU/GPU variant가 바뀌면 동일한 시간 조건을 다시 확인한다.

## 2. 장비 능력·제어 구성별 적용

다음은 binding을 작성할 때 검토할 **구성 유형**이다. 특정 장비가 현재 구현·검증됐다는 지원표가 아니며 모든 유형을 기본 이미지에 포함하라는 요구도 아니다. 실제 지원은 [장비 지원 정책](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md)에 따라 profile별로 선언한다.

| 구성 유형 | 계약 연결 | 필수 확인·제한 |
|---|---|---|
| 고정식 arm과 독립 gripper | FINITE_ACTION과 별도 gripper action | 실제 joint/channel 순서, tool·교정·완료·취소·허용오차, 공유 bus와 자원 |
| arm/gripper가 같은 controller를 공유 | FINITE_ACTION 또는 CONTROL_SESSION | 논리 기능을 독립 controller로 가정하지 않음. SC14 직렬화 또는 검증된 복합 trajectory |
| leader/follower 제어 | CONTROL_SESSION과 LIFECYCLE | source identity·deadman·권한 인계, 활성화의 토크 효과와 지지 |
| 다중 arm·hand·head/lift | FINITE_ACTION과 MODE_TRANSITION | controller/resource 공유, command entry와 독립 actuator 수 구별 |
| 이동 로봇 | CONTROL_SESSION, 별도 이동/도킹 작업 | 속도 접수와 위치 도착·제동·도킹 완료 구별, 주행·팔 동작의 결합 제약 |
| 능동 균형 로봇 | MODE_TRANSITION, CONTROL_SESSION, LIFECYCLE | 자세·안정·접촉·지지·에너지, mode 수락과 물리 후조건 구별 |
| 문·승강기·충전기·시설 제어기 | ENSURE_STATE 또는 FINITE_ACTION | 실제 상태 feedback, 사용 권한·점유·인계, 정지·재부팅·직접 조작과의 경합 |

각 driver의 activate/start/stop/destructor가 장비에 미치는 효과를 검토해야 한다. 종료가 torque disable을 일으킬 수 있는 경우 driver process 종료를 일반적인 hold 구현으로 사용할 수 없다. 실제 배포 profile은 source commit뿐 아니라 launch 조건과 init flag의 **전개된 값**도 포함한다.

## 3. 대표 native 매핑

**JTC**: ROS `FollowJointTrajectory` goal UUID와 RX invocation을 매핑한다. action acceptance, action result, joint observation을 따로 보존한다. 해당 controller가 제공하는 성공의 범위와 tolerance를 profile에 고정한다. joint 목표 도달만으로 소재를 제대로 집었다고 판정하지 않는다. 취소는 특정 goal ID로 수행하고 action result와 실제 정지/지지 조건을 확인한다. topic publish를 action result가 있는 것처럼 포장하지 않는다.

**모드 서비스**: SetMode 성공은 해당 API가 문서화한 수락/변경 결과다. ModeStatus의 신선한 활성 mode와 추가 후조건을 검사한다. ReadyPose를 명령한 뒤 mode 이름만 보고 로봇 자세 완료를 판정하지 않는다. 자세 확보가 필요한 공정은 별도 ENSURE_STATE 또는 명시된 mode completion rule을 사용한다.

**속도/leader/정책 제어**: CONTROL_SESSION의 source identity·sample schema·mode·deadman·expiry reaction을 고정한다. navigation 완료 계약은 위치/도킹 작업의 추가 FINITE_ACTION profile로 설계해야 하며 `cmd_vel=0`과 같은 의미로 사용하지 않는다.

**드라이버 기동/종료**: image health=software ready는 장비 activate 허가가 아니다. prepare에도 torque/initial pose 쓰기가 있으면 LIFECYCLE operation으로 먼저 허가한다. 초기화가 여러 native 효과로 구성되는 upstream 동작이라면 해당 sequence의 단계·결과·복구를 별도 기록 가능한 binding을 준비해야 한다. 이를 할 수 없으면 ‘1 native 호출=내부 여러 효과’의 명시적 복합 native 동작으로 선언하고 부분 결과 UNKNOWN을 허용하며 단계별 exactly-once를 주장하지 않는다.

## 4. 시설·PLC·기존 운영 시스템 연결

| 대상 | binding이 확인할 범위 | 선언하면 안 되는 가정 |
|---|---|---|
| PLC와 설비 제어기 | 명령/결과 식별, 실제 feedback, 통신 설정·I/O 의미·boot·잔류 명령 | 통신 가능 또는 메모리 쓰기 성공을 작업 완료로 간주 |
| 문·승강기·인계 장치 | 출입 권한, 점유·현재 상태·물품 지지, 사람의 직접 조작과 복구 | 기능 이름만으로 안전 interlock과 실제 센서 범위를 추정 |
| 로봇 또는 기존 fleet manager | 접수·실행·결과 조회·취소·소유권 및 물품 인계 의미 | 상위 시스템의 accepted 응답을 최종 서비스 완료로 간주 |
| 부속 제어기 | C08 장비의 능력·관측·boot·명령 경계 | 연결됐다는 이유로 독립 RX Runtime 또는 분산 권한이 검증됐다고 선언 |

PLC의 가능한 handshake 예시는 `request_id/command/parameters → accepted_id/busy/result_id/result_code + 실제 상태 feedback`이다. 실제 장치가 제공하는 기능을 profile에서 확인해야 한다. 숫자 ID를 쓸 수 없으면 edge/level handshake·배타 실행·ready/busy/done reset 순서를 문서화한다. 그 경우 재시작 후 과거 결과 correlation이 제한될 수 있다.

`door_closed`나 `chuck_clamped`는 명령 bit가 아니라 실제 상태 feedback이어야 한다. 메모리 쓰기 성공, 통신 정상, 출력 접점 ON은 물리 완료의 충분조건이 아니다. feedback의 실제 센서 범위·기계적 의미·진단 조건은 해당 공급사 자료와 실물 검증으로 확인한다. 설비·문·그리퍼의 안전 interlock은 RX DB 승인만으로 대체하지 않는다.

## 5. admission과 운영 구성의 전이

구성 lifecycle은 `DRAFT → VALIDATED → RELEASED → INSTALLED → READY_FOR_OPERATION`이다. READY_FOR_OPERATION은 영구 상태가 아니라 현재 장비 조건의 판정이다. release 서명·기술 검증은 검토 역할의 증거이며 현장 교정·mode·관측 신선도를 대신하지 않는다.

1. profile와 site configuration digest·필수 source/artifact·image variant를 검증한다.
2. Host의 실제 device/controller/firmware/mode와 profile을 대조한다.
3. 교정·tool/지그·소재 상태·사람 개입 정책·작업 종류를 검사한다.
4. 필요한 evidence source와 native result correlation·restart·cancel 능력을 검사한다.
5. 충돌 자원을 예약하고 Host grant를 얻는다. 전달 직전 Host가 변할 수 있는 조건을 재확인한다.

실패 결과에는 support_id와 누락 조건을 구조적으로 표시한다. ‘지원 안 됨’ 하나로 합치지 않는다. ProfileFinding의 FindingCode는 0=UNSPECIFIED, 1=SATISFIED, 2=PACKAGE_MISSING, 3=MODEL_MISMATCH, 4=MODE_MISMATCH, 5=CALIBRATION_INVALID, 6=EVIDENCE_UNAVAILABLE, 7=TIMING_UNVALIDATED, 8=AUTHORITY_UNAVAILABLE로 고정한다. RPC Reason의 detail 문자열에서 파싱하지 않는다.

## 6. 두 이미지 명세에 대한 추가 요구

| 이미지 | 추가 의무 |
|---|---|
| rx-platform | durable state volume·backup/restore generation, trusted release/profile cache, 인증서 mount, journal cursor API, STORE_FAULT 진단 |
| rx-solutions | Host별 durable inbox/evidence outbox volume, immutable binding/profile·calibration mount, 로봇 source/stream process 분리, native device 최소 권한 |
| 공통 | software restart는 diagnostic ready로 복원. 기존 작업 자동 재개·torque 활성·새 grant 발급을 healthcheck 성공에 연결하지 않음 |

Host volume은 platform DB와 구별하고 journal ID·peer identity를 백업 manifest에 함께 기록한다. 무조건 `privileged`나 전체 `/dev` mount를 기본 계약으로 요구하지 않는다. 실제 장치·네트워크·RT 권한은 model/variant별 시험에서 확정한다.
