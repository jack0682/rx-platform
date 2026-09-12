# Platform peer 통신 경계

`PlatformIngress`는 실제 TLS listener와 기존 dedicated writer를 연결한다. 로컬 HTTP 실행 파일에 자동 활성화하지 않는다. 이전 `EvidenceIngress`는 호환 alias다. 현재는 composition API이며 제품 두 이미지의 상주 프로세스 설정은 후속이다.

## 제공하는 표면

- `Session.Open`: 등록된 Host/Executor/OPERATOR_API leaf certificate와 peer ID/role, installation/store generation, release, clock, base manifest를 대조한다. 권한은 application의 현재 principal에서 확인한다.
- `Cell.Inspect`: 현재 서비스 session/인증서·셀 협상과 범위에 맞는 기록된 CellContext 조회.
- `Cell.Open`: 같은 인증된 base session에 cell manifest와 CellDefinition/membership을 묶는다. 같은 정의에 여러 셀이 모호하게 대응하면 거부한다.
- `Workflow.GetRun`: 등록된 실행기의 base/cell 협상과 현재 셀 배정을 확인하고, 실제 저장된 RunView/Checkpoint를 반환한다.
- `Cell.BeginPartAttempt`: 명시한 run/mandate/budget와 현재 권한으로 소재 시도를 생성하고 예산 소비·응답을 함께 저장한다.
- `Workflow.ResolveActivation`: 요청 key와 run/node/visit/revision을 확인하여 유일 activation을 생성하거나 기존 mapping을 회수한다.
- `Cell.SubmitOperation`: 유한 run 작업의 전체 cell/base envelope·parent·part·두 CAS를 같은 T1에 연결하고 최초 ADMITTED Receipt를 반환한다.
- `Workflow.CommitCheckpoint`: 현재 권한·전체 요청 동일성·후보 artifact·run CAS·현재 조건을 검사하고 T3와 원래 RunView 응답을 원자적으로 저장한다.
- 선택적 `ExecutorPlan.PrepareCheckpoint`: 별도 binding hash로 분기·대기 전체 상태 후보를 준비한다. 준비는 공정 상태 확정이 아니다. [준비/확정 경계](../../../rx-application/CHECKPOINT_COMMIT.md)를 따른다.
- `Workflow.PauseRun`: explicit executor halt의 run/cell closure 허가 철회와 최종 checkpoint 응답. 물리 취소/정지 완료가 아님.
- `Operation.Reconcile`: 기존 invocation 조회 계획을 영속 병합하고 현재 OperationView를 반환한다. 조회 접수와 결과·자원 해제를 구별한다. [조회·인계 명세](../../../rx-application/RECONCILIATION.md)를 따른다.
- `Operation.Get`: P에 기록된 결과/지식/무결성/자원 처분 조회. 조회로 native 조정/재실행하지 않음.
- 선택적 `ExecutorRead.GetSnapshot/GetArtifact`: 별도 binding hash로 현재 process read cut과 run-owned checkpoint/resolved bytes를 조회한다. [범위와 검증](../../../rx-application/EXECUTION_READ.md)을 따른다.
- 선택적 `Production.Inspect/CompletePart`: 첫 소재 이전부터 완료 이후까지 일관된 run/part/budget 조회와 P 근거에 따른 소재 완료를 제공한다. admission은 frozen BeginPartAttempt를 재사용한다.
- `Cell.RecordProcedure`: 메시지 변환은 있지만 인간 신원 binding은 아직 연결하지 않았다. 서비스 계정에 사람 역할을 추가한 호출은 거부한다. 현재 인간 보고는 직접 terminal HTTPS/개발 HTTP를 사용한다. [범위](../../../rx-application/PROCEDURE_REPORTS.md)를 따른다.
- `Cell.OpenCase/GetCase`: 사건 접수·보류/철회와 조회. 알림 확인·물리 절차·clearance를 혼동하지 않는다. [현재 범위](../../../rx-application/INTERVENTION_CASES.md)를 따른다.
- `Evidence.Publish`: 원본 정보를 보존하는 변환 → 같은 writer의 T2 → durable acknowledgment.
- `Evidence.Get`: producer 소유권·현재 셀 접근권을 확인한 native evidence 원본 조회. legacy 불완전 projection의 원본 재구성은 거부한다.

위 목록 이외의 CellService/Workflow 메서드는 명시적으로 UNIMPLEMENTED다. native 제출은 Cell.SubmitOperation의 현재 허가·조건 검사와 기존 dispatcher/Host gate를 거친다. 공개 base Submit으로 cell 검사를 우회하지 않는다. 공개 P gRPC 전체 구현으로 표시하지 않는다.

실행기의 교체·폐기 boot·P 재시작 처리와 조회의 제한은 [실행기 세션 명세](../../../rx-application/EXECUTOR_PEER.md)에 정리했다. TCP 연결이 존재하거나 세션을 발급받았다는 사실을 가동 준비·운전 허가로 쓰지 않는다.

## 인증과 순서

인증된 producer session은 journal/peer boot/인증 binding과 영속 연결한다. 같은 reconnect는 이전 응답 유실과 관계없이 같은 session으로 수렴한다. 인증 binding이나 Host incarnation이 바뀌면 이전 session을 무효화하고 연결된 기존 운전 권한을 철회한다. 새 session 발급이 새 운전 허가를 뜻하지 않는다.

이 service identity 수명은 P runtime/Host incarnation/현재 등록과 역할에 묶인다. 브라우저 1시간 session 정책을 재사용하지 않는다. TLS 요청마다 등록된 certificate와 session binding을 대조하며, 현재 역할/셀 접근권을 T2에서 다시 검사한다. 동적 certificate registry 변경·제품 credential rotation 관리 흐름은 미완료다.

Publish는 최대128개/1 MiB의 연속 batch다. 같은 seq의 내용 차이에는 native_id/native_data도 포함된다. producer 소유권·원본·현재 결과·원장 위치가 같은 T2에 들어가고, T2가 실패하면 ack를 발급하지 않는다. GAP은 FAILED_PRECONDITION/GAP과 `rx-next-expected` decimal metadata로 알린다. 일부 seq를 건너뛰거나 다른 journal로 자동 초기화하지 않는다.

## 아직 해결해야 하는 cursor 경계

현재 DurableAck의 producer through_seq는 실제 연속 inbox commit 위치이며 Host의 재전송·중복 제거 시험으로 검증했다. platform_cursor는 이제 **별도 control journal의 같은 transaction cut**을 사용하고, 셀 확장의 `site-cell-control-v1` 이름을 쓴다. 내부 감사 사건의 seq를 섞지 않는다.

저장 payload는 typed application 상태 snapshot이다. 전체 CellJournalRecord/EventType/EntityView mapping은 아직 미완료이므로 공개 Journal.Subscribe/GetSnapshot은 등록하지 않았다. [제어 원장 문서](../../../rx-application/CONTROL_JOURNAL.md)의 필수 metadata·checkpoint·wire mapping을 완성하기 전에는 frozen Journal 계약 적합성을 주장할 수 없다. 수신 확인 통과만으로 전체 원장 계약을 VERIFIED로 승격하지 않는다.

현재 지원하는 native-result 외의 증거, artifact 원본 반출, 대규모 journal retention/갭 복구, 전체 P RPC/원장 stream도 후속 범위다.

## 검증

`tools/test_evidence_e2e.sh`는 다른 레포의 test-harness Host 실행 파일을 빌드해 별도 프로세스로 실행한다. 테스트용 journal131개를 연속 전송하며, 첫 data batch를 P가 commit한 직후 ack를 잃게 한다. 131개의 source slot만 저장됨, 마지막 원본 metadata 조회, 미등록 certificate 거부, Host restart 후 동일 journal/cursor 보존과 이전 session 무효화를 확인한다.

이131개는 **합성된 미상관관계 증거**다. P는 알 수 없는 operation을 성공 작업으로 만들지 않고 영향 셀을 차단하며, qualification은 미등록으로 유지한다. 이 시험에서 device effect는0개다. 실제 native T1–Host–T2–handover 경로는 별도 `tools/test_host_e2e.sh`의 두 모의 소재 시도 시험으로 구분한다.

실행기 경로는 `cargo test -p rx-api --test executor_ingress`로 실제 TLS/SQLite에서 검증한다. 양쪽 계약 협상, 인증서/session 결합, 배정 셀, 세션 교체와 자동 실행 금지를 확인하며 장비 명령은 보내지 않는다.

[실행기 요청 명세](../../../rx-application/EXECUTOR_REQUESTS.md)는 전체 payload/key/CAS와 응답 유실 동작을 설명한다. 모의 operator/qualification/Host Arm 후 실제 mTLS로 part/activation을 생성하고 GetRun에서 확인한다. 이 시험에는 장비 제출이 없다.

[실행기 유한 제출/결과 조회](../../../rx-application/EXECUTOR_SUBMISSION.md)의 통합 시험은 양쪽 TLS와 별도 모의 Host에서 수행한다. Submit/Authorize 응답 유실에도 각 slot의 모의 효과가 한 번인지 확인한다.

`Session.Open(OPERATOR_API)`·`Cell.Open`·`Cell.Inspect`를 지원한다. 운영 API는 전송 역할로 인증하며 인간 쓰기/실행기 권한을 받지 않는다. `CellContext`는 기록된 mode/commissioning/block 생성 revision을 사용한다. [모델·호환·남은 인간 binding](../../../rx-application/CELL_CONTEXT_AND_OPERATOR_PEER.md)을 따른다.

인간 ProcedureRecord를 service 계정의 추가 Operator/RecoveryLead 역할로 작성하는 이전 초안 경로는 차단했다. 직접 terminal HTTPS의 사람 신원은 [신원 연결](../../../rx-application/TERMINAL_IDENTITY.md)로 처리한다. gRPC의 OPERATOR_API 사용자 위임은 후속이다.
