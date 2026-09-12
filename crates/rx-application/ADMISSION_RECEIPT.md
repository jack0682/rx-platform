# P의 최초 접수 확인서

`ADMITTED` 응답은 특정 작업이 P에 영속 접수됐다는 확인이다. Host 준비, native 수락, 완료 또는 자원 인계를 뜻하지 않는다.

## 근거 위치

`Journaled`가 새 Work의 ADMITTED/revision1 상태를 control journal에 추가할 때 반환받은 **실제 seq**를 사용한다. 같은 transaction에서 `admissionreceipt/<operation key>`에 operation ID, intent digest, 그때의 operation revision, journal ID와 seq를 기록한다.

이 seq는 나중의 journal head, 내부 audit event 수, 현재 Work record revision 또는 임의 상수가 아니다. 접수 확인서가 가리키는 control record에는 해당 작업의 원래 ADMITTED snapshot이 있다. receipt 기록 실패는 T1의 Work/slot/permit/outbox/요청 결과와 함께 rollback된다.

P control journal ID는 UUIDv8 형태로 다음 의미에 결합한다: `RX-CONTROL-JOURNAL-ID-v1`, installation ID, store generation, `site-cell-control-v1`. 해시는 기존 canonical domain 규칙을 사용한다. 같은 P process가 재시작해도 ID는 같고, 저장 세대나 journal view가 달라지면 달라진다. Host의 delivery/evidence journal ID와 혼용하지 않는다.

## 읽기와 후속 상태

`Engine::admission_receipt`는 현재 session·셀 접근권을 확인하고 Work의 ID/intent와 접수 확인서가 일치하는지 검사한다. 뒤의 Host receipt나 완료 결과를 이 레코드에 덮어쓰지 않는다. 보류·권한 철회·후속 결과가 생겨도 최초 접수 기록은 그대로 남는다.

프로토콜 adapter는 이 값을 frozen base Receipt의 ADMITTED stage로 변환한다. P operation revision은 포함하고, 아직 P 접수 확인서에 속하지 않는 invocation/Host state/cancel ID는 넣지 않는다. 원래 확인서의 journal ID를 현재 프로세스 boot ID로 교체하지 않는다.

이 기능 이전에 만들어진 Work에 원래 접수 위치가 없으면 과거 head나 현재 snapshot으로 확인서를 만들지 않는다. `CONTINUITY_UNPROVEN`으로 남기며, 그것이 기존 작업을 새 ID로 재전송할 근거가 되지 않는다. 신규 T1 경로에는 이 확인서를 함께 저장한다.

## 시험과 한계

저장 직전 실패와 저장 후 응답 유실을 주입한 뒤 동일 요청을 회수하고, 접수 확인서 하나만 남는지 확인했다. 그 seq로 control record를 읽어 operation ID/revision/ADMITTED 상태와 native invocation 부재를 대조한다. 뒤의 Hold에도 확인서 bytes가 유지되고 P 재시작/저장 세대 변경의 journal identity 규칙이 맞는지 확인한다.

공개 [Cell.SubmitOperation의 유한 run envelope/Receipt 반환과 Operation.Get](EXECUTOR_SUBMISSION.md)을 연결했다. base Operation.Lookup과 전체 Journal API는 후속이다. 접수 원장 위치를 확보한 것을 native 결과 증거 또는 전체 공개 원장 구현으로 표시하지 않는다. 저장소 복원 시 과거 journal namespace/기록의 보존·조회 정책도 별도 구현이 필요하다.
