# 제어 상태 원장과 감사 기록

저장 schema3부터 두 순번을 분리한다.

| 저장소 | 역할 |
|---|---|
| `events` | application 내부 감사/진단 기록. Host에서는 기존 evidence outbox journal로 계속 사용 |
| `control_events` | P의 commit된 제어 상태 변화, 독립적 연속 seq |
| `control_entities` | 같은 commit에서 갱신하는 현재 제어 상태 projection |

`site-cell-control-v1`은 셀 확장의 통합 원장 식별자다. base/cell 통합 기록을 제공해야 하므로 이전의 `site-control-v1` 이름으로 새 제어 stream을 열지 않는다. base stream을 일부 사건만 거르는 방식으로 제공해서도 안 된다.

## 저장 경계

`Engine`는 `Journaled<R>`를 통해 transaction을 수행한다. wrapper는 셀·실행·작업·시작 시도·mandate·part·permit·resource의 실제 `put`을 추적한다. 특정 handler가 별도 event 함수를 호출하는 것을 잊어도 상태 변경이 원장에서 빠지지 않도록 한다. 핵심 entity key에 다른 schema를 덮어쓰면 거부한다.

하나의 transaction에서 같은 entity를 여러 번 바꾸면 최종 상태 한 개를 기록한다. 원본 entity 변경, idempotency/outbox, 제어 사건, 현재 projection은 원래 repository transaction 안에서 함께 commit/rollback한다. 감사 기록을 제어 사건으로 바꾸어 계수하지 않는다.

내부 `ControlChange`는 entity 종류·변경 종류·그 시점의 immutable Record와 관련 evidence ID를 담는다. T2는 wrapper가 제어 기록을 마친 뒤 같은 transaction의 control head를 반환한다. 따라서 ack가 이전 head나 나중의 다른 transaction 위치를 가리키지 않는다. 같은 inbox batch의 중복 회수는 새 제어 변경을 만들지 않는다.

첫 journal 초기화에서 이미 존재하는 지원 entity는 `SNAPSHOT_SEED`로 기록한다. 과거 사건을 새 ADMITTED 사건으로 위장하지 않는다. 초기화 marker는 한 번만 기록한다. SQLite schema migration은 기존 audit/evidence/outbox를 보존하며, control history가 없던 시기의 세부 사건을 복원했다고 주장하지 않는다.

원장 내 순서는 DB commit/저장 순서다. 같은 transaction 안 entity 순서는 안정적인 key 순서이며 장비의 물리 발생 순서가 아니다. 외부 운전 권한 판단은 항상 application의 현재 상태에서 수행한다.

신규 Work의 ADMITTED control record 위치에는 [최초 접수 확인서](ADMISSION_RECEIPT.md)를 함께 저장한다. 접수 seq는 실제 control INSERT의 반환값이며, 이후의 head나 native 결과로 덮어쓰지 않는다.

## 현재 구현과 공개 계약의 경계

현재 저장된 payload는 **typed application 상태 snapshot**이다. repository에서 연속 사건과 같은 cut의 projection을 다시 읽는 것까지 구현했다. `CellJournalRecord`/`SnapshotEntity` wire 표현 전체를 완성한 상태는 아니다.

공개 mapping에 필요한 후속 사항은 다음과 같다.

- CellContext의 mode·commissioning, Block.created_revision 등 명시적 상태/출처 보존.
- embedded Qualification의 독립적인 변경·상태·limitations 표현.
- RunView의 checkpoint artifact와 activation/slot snapshot은 [같은 cut에 저장](CHECKPOINT_ARTIFACT.md)하도록 연결했다. 이전 journal schema와 전체 공개 stream migration은 남아 있다.
- Mandate/Permit의 관련 run/budget/condition 정보가 그 사건의 cut에 고정되도록 하는 표현.
- 현재 지원 entity 외 case/procedure/clearance/material/change/restart preparation 모델과 projection.
- 설치 전체 접근권, paged snapshot 수명, exclusive cursor·retention/gap, bounded subscriber와 SSE.

이 값들을 현재 DB에서 뒤늦게 끌어와 과거 event에 붙이거나 임의 기본값으로 채우지 않는다. 전체 mapping이 완성되기 전에는 공개 Journal/Snapshot RPC를 활성화하지 않으며, R17은 PARTIAL이다. 현재 publisher는 producer through를 영속 재전송 위치로 사용하고 public journal을 구독하지 않는다.

## 검증

- control event INSERT 뒤 projection 갱신을 실패시켜 core entity/control event/projection이 모두 rollback되는지 확인.
- 감사 append와 user login이 control seq를 변경하지 않는지 확인.
- T2 응답 유실/중복에도 하나의 evidence 관련 제어 변경만 저장되는지 확인.
- Hold의 직접 `tx.put` 변경도 work/resource/cell projection에 포함되는지 확인.
- source seq가 연속이고 `after`가 exclusive인지, snapshot head와 event tail이 일치하는지 확인.

이는 저장·capture 계층의 증거이며, 아직 구현하지 않은 공개 wire mapping이나 전체 제품 규모의 성능 검증을 대신하지 않는다.
