# 분기·대기 후보 준비와 체크포인트 확정

상태: P 내부 전이, Runtime typed command, 선택적 PrepareCheckpoint와 확정 계약의 Workflow.CommitCheckpoint를 구현했다. S의 분기/대기 worker·영속 요청·관측 복원도 후속 단계에서 연결했다. [실행기 검증 범위](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-executor/DECISIONS_AND_RECOVERY.md)를 함께 읽는다.

## 완전한 상태 후보를 사용한다

확정된 ChangeCheckpoint는 상태 artifact와 기존 activation/slot 연결을 운반한다. 이를 임의 명령 봉투로 바꾸지 않는다. PrepareCheckpoint가 현재 P 상태에서 다음 `rx.executor-state.v1` 전체 후보를 만들고, 실행기는 그 Checkpoint를 그대로 CommitCheckpoint에 제안한다.

후보는 run의 metadata·purpose·budget·session·part ID와 전체 activation/slot binding을 유지하고, 한 visit/node의 분기 선택·대기 시작·대기 결과만 제안한다. 새 revision은 current+1이다. 후보 artifact는 내용 주소로 보존하며 run 소유 artifact 경로에서 읽을 수 있지만, 현재 RunView를 교체하지 않는다. 읽을 수 있는 후보·과거 artifact는 현재 실행 허가가 아니다.

## 준비와 확정의 책임

| 단계 | P가 확인하는 것 | 원자적으로 바뀌는 것 |
|---|---|---|
| 준비 | 현재 실행기 권한/셀 배정, run/visit/node, frontier, P 관측과 시각 | 후보 artifact·소유 참조·준비 기록. Run/공정 결정/작업은 그대로 |
| 대기 중 | 관측이 UNKNOWN이거나 wait 조건이 아직 충족되지 않음 | 결정·후보를 만들지 않고 WAITING 반환 |
| 이미 확정 | 해당 branch/window/result가 이미 저장됨 | ALREADY_APPLIED 반환. 실행기가 snapshot을 새로 읽음 |
| 확정 | 현재 인증 → 전체 요청 key/body → run CAS/후보 소유·수명·세대 → 현재 운전 적격성 → 상태 재계산 | process checkpoint, Run revision, 현재 artifact/제어 사건, 원래 응답을 같은 commit에 기록 |

기존 내부 choose/start/check 경로와 후보 준비·확정은 `process_transition.rs`의 같은 전이 함수를 쓴다. 공정 체크포인트 저장도 다음 revision을 확인하는 한 함수를 사용한다. 통신 handler에는 별도 조건 판단이나 SQL을 넣지 않았다.

## 오래된 판단은 그대로 적용하지 않는다

후보에는 P runtime boot, executor session, cell epoch/scope, base revision, P 준비 시각과 최대 100ms 유효 기간이 결합된다. 확정할 때 현재 사실로 상태를 다시 계산하고, artifact의 전체 bytes와 기존 ID 연결을 대조한다. run revision이 같아도 관측 근거가 바뀌면 후보가 거부될 수 있다. 권한 철회·새 session·새 boot의 후보는 운전을 되살리지 않는다.

분기의 bool이나 완료 상태를 caller가 자유롭게 제출할 수 없다. 업로드 API도 제공하지 않는다. P가 준비한 artifact의 정확한 참조와 명세를 사용하고, 현재 평가와 같은 상태여야 한다. 잘못된 schema/size/revision, 다른 run, 추가·삭제·교체한 activation/slot은 허용하지 않는다.

## 대기 시각

대기 시작 후보의 started_at과 deadline은 최초 P 준비 시각으로 정한다. 성공한 commit이 그 window를 확정하며 네트워크 지연만큼 시간을 추가하지 않는다. 일단 확정된 window는 다른 key로 재시작해도 늘어나지 않는다. 만료된 후보를 버리고 새 준비를 하는 것은 아직 확정되지 않은 시도의 교체다.

조건 만족 후보가 있어도 실제 확정 시각이 deadline 이상이면 만족 상태를 적용하지 않는다. 새 준비에서 TIMED_OUT을 제안한다. timeout은 native 정지·작업 완료·자원 인계와 무관하다. clock 연속성을 확인할 수 없으면 확정하지 않는다.

## 응답 유실과 보존

같은 key/body로 이미 확정된 변경을 요청하면 현재 revision으로 응답을 꾸미지 않고 최초 RunView를 반환한다. 현재 역할·셀 접근권은 재생 전에도 검사한다. 저장 전 실패는 공정과 revision을 바꾸지 않는다. 저장 후 응답 유실은 사실을 취소하지 않는다.

prepared artifact가 현재 checkpoint로 받아들여질 때, 자동으로 생성한 최종 artifact의 digest까지 일치해야 transaction을 완료한다. 후보 자체나 현재 상태의 다른 artifact로 최종 응답을 대체하지 않는다.

## 통신 및 검증

PrepareCheckpoint는 [별도 명세와 binding hash](../../spec/executor-plan/v1/README.md)를 요구한다. 기존 base/cell 규범 8개와 executor-read binding은 유지했다. CommitCheckpoint는 기존 frozen protobuf와 body expected_revision 규칙을 그대로 사용한다.

core 시험은 후보의 비적용, 준비/확정 rollback과 응답 유실, 동일 key/전체 body 충돌, current-role 검사, artifact·mapping 변조, 관측 변화, 후보 만료, 대기 deadline 경계와 window 보존을 확인한다. 실제 mTLS 시험은 분기 및 대기 준비→확정→artifact 조회와 확정 응답 유실 후 회수를 확인한다. 기존 유한 작업·Pause·인계·복원 경로도 회귀 검증한다.

S의 분기/대기 영속 요청과 결과 관측, 구조화된 체크포인트 거부에 따른 재준비를 연결했다. 남은 기능은 상주 BT loop, 개입/clearance와 복구, 후보 정리·긴 run의 용량/index, 전체 운영 UI와 제품 배포다. 현재 단일 상태 artifact의 크기 제한과 scan 비용을 해소한 것으로 간주하지 않는다. 시계·관측은 모의 fixture이며 물리 셀은 NOT_COMMISSIONED다.
