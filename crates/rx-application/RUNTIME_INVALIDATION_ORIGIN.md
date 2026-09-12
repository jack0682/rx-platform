# Runtime 재시작 제한의 출처

`Engine::open`은 기존 셀에 RuntimeRestart 제한을 기록하는 같은 transaction에서 `runtimeinvalidationorigin` 원본을 추가한다. 키는 정확한 block ID에 결합하고 schema는 `rx.runtime-invalidation-origin.v1`이다. 생성한 record는 revision1로만 읽으며 기존 원본을 수정하거나 과거 제한에 출처를 합성하지 않는다.

`RuntimeInvalidationOrigin`은 installation/store generation, cell, 원래 Block snapshot, 이전·현재 Runtime boot, 이전·현재 cell revision/epoch/scope vector/구성 digest를 보관한다. 구성 digest는 기존 process-change의 configuration reference와 같은 canonical 원문 SHA-256이다. 기록 시 새 제한이 정확히 한 개이며 이전 제한이 그대로 남아 있고 revision/epoch/scope가 한 번 증가했는지 검사한다. 기동 transaction 실패면 제한·origin·새 boot·Fence 기록도 함께 rollback된다.

`runtime_invalidation::load(tx, block_id)`는 역사적 원본과 schema/key/revision/구조를 확인한다. `read_for_cell(tx, installation, cell_id, block_id)`는 현재 저장 cut의 설치 신원·generation·원래 block snapshot과 비후퇴 revision/epoch/scope를 추가 대조한다. 출처가 없으면 None이다. 앞선 boot의 origin도 같은 block이 남아 있으면 읽을 수 있다. 현재 구성이 과거 origin과 다를 수 있으므로 적용 before/after·검증 계획 적합성은 호출자가 별도로 검사한다. 원본 참조는 `digest()`의 `RX-RUNTIME-INVALIDATION-ORIGIN-v1` digest를 사용한다.

이 기록은 Runtime이 제한을 생성한 사실이다. 빈 P 원장이나 origin 존재가 Host/native 작업·장비·소재·인원 상태의 근거가 되지는 않는다. 원본의 조회 자체는 block 해제·qualification·Run 시작을 수행하지 않는다. 출처 없는 과거 RuntimeRestart와 OperatorHold 등 다른 제한은 그대로 남는다. SDK·공통/셀 규범·UI에는 이 단계에서 변경이 없다.

명시적 선택을 위한 읽기 API는 `GET /api/v1/runtime-restrictions?cell=...`다. 현재 Engineer/Verifier/ReleaseManager 중 하나의 역할과 요청 셀 접근권을 요구하며, 등록 단말 세션은 기존 단말 권한 교집합을 적용한다. 한 transaction에서 현재 설치와 셀을 확인하고 latched RuntimeRestart만 ID 순서로 반환한다. 응답은 `{cell, revision, epoch, configuration_digest, restrictions:[{block, origin, origin_digest}]}`이며 원본이 없으면 두 origin 필드를 모두 null로 유지한다. 현재 origin이 일치하지 않거나 훼손되면 없는 것으로 바꾸지 않고 조회를 거부한다. 조회는 origin·소유 결합·block·qualification을 생성하거나 변경하지 않는다.

## 재검증 요청과 최종 활성화의 연결

`Requalification.Begin.runtime_restrictions`는 해제 대상으로 검토할 block ID와 정확한 origin digest의 명시적 map이다. 최대128개이며 요청한 영향 셀에 속해야 한다. origin의 구성 digest는 해당 Change가 실제 적용한 before 또는 after 구성과 같아야 한다. 과거 출처를 reason 이름만으로 만들어 붙이거나 현재 제한을 일괄 선택하지 않는다.

신규 요청은 v2이며 원래 origin snapshot과 전체 영향 digest를 포함한다. request→report digest→서명에 그대로 결합된다. v1 기록은 기존 직렬화와 digest를 유지한다. Begin은 Change·Job·request digest·origin digest에 묶인 소유 기록을 원자적으로 추가하지만 제한을 해제하지 않는다. 원래 활성 자격의 Change에 재시작 제한을 기록하는 경우에도 같은 transaction의 origin digest를 보존한다.

6개 검증 영역의 보고서, 독립 검토, 발급, 모든 Host 확인, 최종 활성화의 기존 절차가 필요하다. `owned_clear`는 현재 Job이 선택한 origin과 그 Job의 불변 결합을 모두 대조한다. J1에서 선택했다는 이유로 선택 없는 J2가 제한을 해제할 수 없다. commit 전후 장애에서 같은 request key는 같은 결과를 회수하며 부분 소유 기록을 남기지 않는다.

영향 검사는 모든 대상 셀의 Host·resource를 seed로 공유 관계의 전체 닫힌 집합을 다시 계산한다. 대상 셀 집합과 구성/공유 영향 digest는 대기 중 요청과 활성 자격 양쪽에서 검사한다. 원래 셀 revision이 그대로여도 새 공유 셀이 생기면 과거 자격을 현재로 사용할 수 없다.

시험은 최초 기동→공정 적용→명시적 선택→서명 보고서→독립 승인→Host 확인→활성화와 선택 누락/변조/다른 Job 재사용/공유 영향 변경을 검증한다. 활성 상태에서 P를 재시작한 뒤 새 Begin에 진입하는 경로도 확인하지만 Host producer의 실제 재연결·재등록과 전체 재활성화는 아직 별도 미완료 경로다. 실제 장비나 소재 상태는 검증하지 않았다.
