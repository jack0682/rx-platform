# P의 공정 결정·checkpoint

`CellConfiguration.process`가 있는 경우 P는 shared resolved process를 직접 검증한다. Recipe ArtifactRef의 digest/schema/size와 실제 graph bytes, operation node 집합과 StepBinding Host/Intent를 대조한다. 별도 predecessor 목록과 graph 규칙을 동시에 해석하지 않도록 graph mode의 predecessor 목록은 비운다.

기존 process=None finite 설정은 호환용 경로이며 branch/wait 기능을 제공하지 않는다. Graph 설정을 단순 목록으로 내려서 조건을 우회하지 않는다.

## 권위와 저장

ProcessCheckpoint는 run+visit에 속하고 branch 결정, wait window/result, decision 시각을 보관한다. PRODUCTION의 visit은 실제 part ordinal, SETUP은1이다. Repeat/Call은 compiler가 고유 node identity로 펼치므로 같은 원본 단계가 서로 다른 반복 위치에서 충돌하지 않는다.

- `choose_process_branch`: 현재 executor/run 권한과 revision, 현재 frontier를 확인한다. 실제 저장된 fact/quality/source generation/age로 평가하여 PASS/FAIL일 때만 선택을 기록한다. UNKNOWN은 선택하지 않는다. 같은 node의 선택은 이후 fact가 바뀌어도 바꾸지 않는다. 새 작업의 actuation 조건은 T1에서 별도로 다시 확인한다.
- `start_process_wait`: P의 시각으로 시작/마감을 고정한다. 다른 key로 다시 요청해도 같은 window를 회수하며 deadline을 늘리지 않는다.
- `check_process_wait`: P가 현재 조건과 deadline을 평가한다. 결과는 decision ID/근거/시각과 기록한다. caller의 시각·PASS bool은 받지 않는다. timeout은 native 정지/작업 완료가 아니다.
- `process_progress`: P의 저장 상태로 완전한 ProgressView를 만들고 frontier를 계산한다. admission_allowed는 현재 run/executor 권한과 별도이며, 읽을 수 있다는 사실이 실행 허가가 되지 않는다.

결정과 checkpoint/run revision·idempotency 결과는 같은 transaction이다. 제어 원장도 checkpoint 변경을 포착한다. 현재 명시적 intervention continuation은 제공하지 않으므로 해당 frontier는 block 상태를 유지한다. 임의 clearance ID 입력으로 통과하는 P API는 없다.

## 활성화와 완료

ResolveActivation과 새 Submit slot의 T1 모두 P frontier에서 허용한 operation node인지 확인한다. Unselected branch, 아직 완료되지 않은 wait 또는 predecessor를 건너뛰어 활성화하지 못한다. Existing slot/key 회수와 새 admission은 구별한다.

Part 완료는 선택된 graph의 완료와 각 작업의 결과/인계를 검증한다. 선택하지 않은 branch 작업을 만들도록 요구하지 않는다. P restart 후 checkpoint/결정은 남지만 run은 RECOVERY_REQUIRED이고 admission_allowed는 false다. 새 session으로 과거 실행 권한을 되살리지 않는다.

## 현재 외부 연결 범위

이 API는 실제 application transaction과 Runtime의 typed Command에 연결했다. [내용 주소 checkpoint artifact와 RunView 읽기](CHECKPOINT_ARTIFACT.md)를 같은 transaction/로컬 API에 연결했다. [후보 준비와 외부 Workflow.CommitCheckpoint](CHECKPOINT_COMMIT.md)의 payload/CAS와 실제 mTLS를 연결했다. 내부 경로와 후보 확정은 같은 상태 전이·체크포인트 저장 함수를 사용한다. S Frame·유한 작업/Pause/인계 worker는 연결됐고, 분기/대기 worker와 관측 복원도 연결했다. 전체 CellJournal mapping·명시적 재시작 절차는 미완료다.

진행 상태 구성과 상관관계 조회는 아직 entity scan을 사용한다. 대규모 index/성능 검증과 완전한 recovery/restart/clearance 모델이 남았다. 이 구현은 전체 납품 또는 물리 장비의 운전 검증을 의미하지 않는다.

## 검증

P 시험은 미선택 branch 활성화 거부, 결정 응답 유실 뒤 동일 결과 회수, 조건 변화 후 기존 선택 유지, wait deadline 재설정 거부·실제 P timeout/성공, recipe binding 교체 거부, restart 후 decision 보존·authority 철회, 선택한 branch와 인계만으로 part 완료를 확인한다. 장비 결과와 인계는 명시적 simulation fixture다.
