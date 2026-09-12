# 장비 binding과 실행 허용 조건

규범: RX 계약 v1.0 · 입력: [자사 지원표](../../13_robotis_support_matrix.md), [이미지 명세](../../14_image_support_spec.md)

## 1. Profile의 역할

공통 계약은 완료/불명/권한의 의미를 정의한다. BindingProfile은 그 의미를 **특정 모델·모드·controller·펌웨어·교정·native API**에 연결한다. ‘OMY 지원’ 같은 제품군 이름 하나로 서로 다른 controller 구성을 합치지 않는다.

Profile은 불변 artifact이며 내용 digest로 참조한다. 필수 항목이 없는 profile은 기본 지원 backlog에는 남지만 생산 admission에는 사용할 수 없다. 모델명만 맞고 controller/firmware/revision이 다르면 별도 profile이다.

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

## 2. 자사 모델·모드별 적용

다음은 공통 계약에 대한 **필수 binding 설계 의무**다. 실제 지원 검증 완료표가 아니다. 아래 모든 ID는 solutions 기본 지원 범위이며 구현에서 선택 설치 대상으로 낮출 수 없다.

| 지원 ID·구성 | 계약 연결 | 필수 확인·제한 |
|---|---|---|
| OM-01 OpenManipulator-X, OM-02 OMX-F general | FINITE_ACTION trajectory + 별도 gripper action | 각 arm joint 수/순서, tool·그리퍼 결과, 취소/허용오차. 같은 physical bus 영향 별도 확인 |
| OM-03 OMX-F AI follower | FINITE_ACTION, 필요 시 CONTROL_SESSION | gripper 포함 6개 JTC command entry. gripper를 독립 controller로 가정하지 않음 |
| OM-04 OMX-L leader | CONTROL_SESSION + LIFECYCLE | gravity/trigger/command broadcaster, leader 관측→follower source 연결, torque·지지·deadman |
| OM-05 OMY-3M general | FINITE_ACTION trajectory | 6 arm JTC, 별도 gripper API 있다고 추정 금지 |
| OM-06 OMY-F3M general | FINITE_ACTION arm + 별도 gripper | 6 arm JTC와 gripper action. 장비 조합별 tool/calibration. 첫 현장 선정 완료를 뜻하지 않음 |
| OM-07 OMY-F3M follower AI | FINITE_ACTION / CONTROL_SESSION | 7개 command entry의 공유 JTC, GPIO. SC14 직렬화/복합 trajectory 적용 |
| OM-08 OMY-F3M leader AI | CONTROL_SESSION + LIFECYCLE | effort gravity+command broadcaster, leader 활성 자체의 토크 효과 |
| OM-09 OMY-L100 leader | CONTROL_SESSION + LIFECYCLE | gravity/spring/broadcaster, follower 세션과 authority 인계 |
| OM-10 L100 follower | FINITE_ACTION / CONTROL_SESSION | gripper 포함 7개 JTC entry, controller resource 공유 |
| FFW-01 F1, FFW-02 BG2 rev2/3/4 | 양팔 FINITE_ACTION, head/lift 별도 자원 | 각 revision 별 profile. arm당 gripper 포함 8개 entry; 독립 gripper 덮어쓰기 금지 |
| FFW-03 BH5 rev1 | arm trajectory, hand JTC/effort 모드, head/lift | arm당 7개, hand당 20 command entry를 20 독립 actuator로 해석하지 않음. controller switch는 MODE/LIFECYCLE 조건 |
| FFW-04 F2, FFW-05 SG2 rev1 | arm/head/lift + base CONTROL_SESSION | steering 초기화와 robot manager mode, swerve base. 속도 수용은 위치·도킹 완료가 아님 |
| FFW-06 SH5 rev1 | 위 구성 + hand/pressure observation | 손 mode·압력 데이터 provenance, 물체 지지/놓기 후조건 |
| FFW-07 LG2 leader | 양쪽 CONTROL_SESSION + LIFECYCLE | 좌/우 serial 역할, spring/joystick/broadcaster, source 단절·소유권 변화 |
| FFW-08 mobile-base component | CONTROL_SESSION | launch 인자·SG2 config·URDF·controller 정합성을 검증하기 전 실제 base 지원 완료 선언 금지 |
| AS-01 AI Sapiens K1 rev1 | CONTROL_SESSION, LIFECYCLE | 23개 impedance command entry, positions/feedforward/kp/kd schema. joint-state/IMU/RC·UDP 역할·토크 효과 검증 |
| AS-02 K1 sim2real | MODE_TRANSITION + CONTROL_SESSION | Damping/ReadyPose/Velocity mode API, warmup/권한·ONNX artifact. mode service 수락과 자세/안정 상태를 구별 |

모든 DHI 계열은 현재 source의 `on_activate→start`, `stop`, destructor 효과를 검토해야 한다. 정상 종료에서 torque disable이 실행되는 경로가 있으므로 driver process를 죽이는 것을 일반적인 hold 구현으로 사용할 수 없다. 실제 배포 profile은 source commit뿐 아니라 xacro 조건과 init flag의 **전개된 값**도 포함한다.

## 3. 대표 native 매핑

**JTC**: ROS `FollowJointTrajectory` goal UUID와 RX invocation을 매핑한다. action acceptance, action result, joint observation을 따로 보존한다. 해당 controller가 제공하는 성공의 범위와 tolerance를 profile에 고정한다. joint 목표 도달만으로 소재를 제대로 집었다고 판정하지 않는다. 취소는 특정 goal ID로 수행하고 action result와 실제 정지/지지 조건을 확인한다. topic publish를 action result가 있는 것처럼 포장하지 않는다.

**모드 서비스**: SetMode 성공은 해당 API가 문서화한 수락/변경 결과다. ModeStatus의 신선한 활성 mode와 추가 후조건을 검사한다. ReadyPose를 명령한 뒤 mode 이름만 보고 로봇 자세 완료를 판정하지 않는다. 자세 확보가 필요한 공정은 별도 ENSURE_STATE 또는 명시된 mode completion rule을 사용한다.

**속도/leader/정책 제어**: CONTROL_SESSION의 source identity·sample schema·mode·deadman·expiry reaction을 고정한다. navigation 완료 계약은 위치/도킹 작업의 추가 FINITE_ACTION profile로 설계해야 하며 `cmd_vel=0`과 같은 의미로 사용하지 않는다.

**드라이버 기동/종료**: image health=software ready는 장비 activate 허가가 아니다. prepare에도 torque/initial pose 쓰기가 있으면 LIFECYCLE operation으로 먼저 허가한다. 초기화가 여러 native 효과로 구성되는 upstream 동작이라면 해당 sequence의 단계·결과·복구를 별도 기록 가능한 binding을 준비해야 한다. 이를 할 수 없으면 ‘1 native 호출=내부 여러 효과’의 명시적 복합 native 동작으로 선언하고 부분 결과 UNKNOWN을 허용하며 단계별 exactly-once를 주장하지 않는다.

## 4. 레이저 열처리기와 타사 연결

| 대상 | 현재 확인 | v1 연결과 미확정 |
|---|---|---|
| 레이저 열처리 PLC | Mitsubishi Q03UDVCPU, QJ61BT11N, 아날로그 모듈·CC-Link I/O 자료 | C08 PLC binding. MC Ethernet 가능성은 실제 enable/IP/port/PLC program/주소 합의와 별개. Door/Chuck ENSURE_STATE 후보. 실제 신호 미확정 |
| MCT-2 | Siemens SINUMERIK/SINAMICS 사진 | NCU 세부 모델·통신 옵션·OEM 프로그램 미확정. 다른 장비 FANUC HMI와 혼합하지 않음 |
| UR5e, FR3 | 로봇 후보 | SDK/action 별 접수·결과·캐시·mode·중단 profile 필요. 첫 투입 모델 선정 완료 아님 |
| 외부 미니컴퓨터 | 주 컴퓨터에 연결되는 부속 제어기 역할 | C08 장비로 능력/관측/boot/명령을 정의. 독립 RX worker·분산 Runtime으로 취급하지 않음 |

레이저 PLC의 제안 handshake는 `request_id/command/parameters → PLC accepted_id/busy/result_id/result_code + 실제 상태 feedback`이다. 이는 **OEM과 설계할 목표**이며 현재 PLC 프로그램에 존재한다고 주장하지 않는다. PLC가 숫자 ID를 쓸 수 없으면 edge/level handshake·배타 실행·ready/busy/done reset 순서를 문서화해야 한다. 그 경우 재시작 후 과거 결과 correlation이 제한될 수 있다.

`door_closed`나 `chuck_clamped`는 명령 bit가 아니라 실제 상태 feedback이어야 한다. CPU 메모리 쓰기 성공, CC-Link 통신 정상, 출력 접점 ON은 물리 완료의 충분조건이 아니다. feedback의 실제 센서 범위·기계적 의미·진단 조건은 OEM 문서로 확인한다. 레이저/문/척의 안전 interlock은 RX DB 승인만으로 대체하지 않는다.

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
