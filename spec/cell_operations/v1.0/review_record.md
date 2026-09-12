# 셀 운영 계약의 독립 문서 검토 기록

2026-09-10 · 검토 종류: 독립 AI 독자 검토 및 통합 문서 검토. 실제 제작팀/OEM의 합의·서명, 법적 적용 판단, 코드·실물·기능안전 검증을 수행한 기록이 아니다.

## 검토 범위

의미 독자는 01–03과 조사/선택 근거를 읽고 정상 자동 운전·보류/철회·사람·지지·복구의 반례를 검토했다. protocol 독자는 01–04와 필요한 base 계약을 읽고 독립 구현의 CAS·epoch·permit·clearance·unique slot·원장/호환 해석을 검토했다. 주 작성자는 05의 trace 및 N01–N06/OI01–18/OV01–11과 전체 문서의 연결을 대조했다.

| 발견한 지적 | 반영과 검토 결과 |
|---|---|
| 단순 응답 유실을 TRANSIENT에 포함하지 않은 정의 | 무개입·무보호반응·설명 가능한 정상 효과/동일 invocation·연속성 조건을 명시 |
| DIAGNOSTIC에서 정상 집기 등 물리 변화까지 금지 | 원 invocation으로 설명되지 않는 변화만 금지하도록 정정 |
| SR02의 모든 관측 단절에 대한 blanket mandate 폐기 | 연속성 상실/보호 반응과 단순 보류를 구별 |
| 재시작의 대상 case가 자기 시작을 막음 | 지정 READY cases만 clearance 소비/새 mandate commit과 원자 종료, 다른 제한은 유지 |
| 새 fence가 이전 clearance epoch를 무효화 | PrepareRestart에서 fence/재확인 후 그 epoch의 clearance 발급, RestartRun은 같은 epoch 사용 |
| READY 전이가 clearance의 case revision을 무효화 | 전이 후 case revision과 clearance를 같은 transaction에 기록 |
| 오래된 vector/Arm이 epoch·새 block을 되돌림 | Host durable 최대값·exact vector·명시 block ID·같은 gate 규칙 |
| 그리퍼/척이 서로 지지를 보고 동시에 해제 | 같은 소재/인계의 support resource를 base 전역 예약 CAS에 포함 |
| part 수량과 node activation 수가 혼용 | PART_ATTEMPT/OPERATION_COUNT·유일 part ID·run 예산 원장, 마지막 part의 남은 node 허용 |
| 기존 part가 새 mandate에 속하지 못함 | part의 run 소유와 restart_plan에 따른 continuation 검사 |
| 비운전 종료에 가짜 restart plan이 필요 | PrepareClose 경로와 REMAIN_OUT_OF_SERVICE clearance/원자 close+latch |
| Host의 transient 해제 전달이 없음 | TRANSIENT는 P 소유, H의 current condition과 LATCHED는 별도 |
| recovery visit의 새 key 재실행 | case/plan/step/visit의 영속 unique slot, 다음 visit edge/근거 검사 |
| extension에서 base run CAS를 전달할 수 없음 | expected_run_revision/expected_case_revision과 nested context의 내부 CAS 매핑 |
| 원장 outer/nested cursor·SSE envelope 모호 | 동일 cursor와 CellJournalRecord data/BASE_EVENT·CELL_EVENT 명시 |

두 독자는 해당 수정과 직접 연결된 부분을 다시 읽었고, 제기한 지적이 해소됐으며 남은 P1/P2 모순을 발견하지 못했다고 보고했다. 이는 유한한 문서 검토의 결과이며 모든 실행 순서에 대한 수학적 증명은 아니다.

통합 검토에서 실제 외부 사실 보고를 오래된 UI CAS 때문에 버리지 않는 규칙, clearance의 target run/envelope/restart plan 결합, Host Inspect를 통한 rollback 비교, 셀/협상 session의 구분, 조건 집합 누락 금지, timing 상한, 구조적 위험 감소와 제어 안전기능의 구별도 확인했다.

## 완료 범위

셀 운영 계약의 책임·상태·전이·부재/거부 조건·기록·메시지·호환·반례와 후속 검증 의무를 문서로 정리했다. 실제 첫 설치의 입력은 NOT_COMMISSIONED로 남는다. 이 문서 검토 결과를 qualification 실물 시험자료로 등록하지 않는다.
