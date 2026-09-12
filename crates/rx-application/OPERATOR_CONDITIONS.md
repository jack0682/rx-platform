# 운영 화면의 조건·관측 진단

2026-09-11. 운영자가 실행을 준비할 때 어떤 조건과 근거를 확인해야 하는지 표시한다. `GET /api/v1/overview`의 각 셀에 `diagnostics`를 추가했으며, 이 조회는 운전 허가·qualification·Arm·native 명령을 생성하지 않는다.

## 하나의 조회와 현재 접근권

기존 overview와 같은 transaction에서 셀 구성, Host 등록, source 관측과 조건식을 읽는다. 현재 계정·단말의 접근 범위로 먼저 셀을 제한하며 다른 셀의 관측과 조건을 응답에 넣지 않는다. 조회마다 권한을 다시 검사한다. `snapshot_id`는 이 읽기의 식별자이며 제어 원장 cursor가 아니다.

새 모델은 `rx.cell-diagnostics.v1`이다. frozen `ConditionEvaluation`의 condition ID/revision 또는 clearance를 새로 만들어 제공하지 않는다. `Start//1`, `Operation/step/0/1` 같은 path는 현재 구성에 연결된 표시 경로다. 공식 조건 계약의 ID로 재사용할 수 없다. 기존 base/cell 규범, 네 optional binding, SDK79개는 유지한다.

## 표시하는 서로 다른 상태

| 항목 | 근거와 의미 |
|---|---|
| 운전 자격·차단·개입 | 기존 CellSummary. 조건이 충족돼도 자격 미등록·기존 차단은 함께 표시 |
| 장비 연동 권한 | 저장된 Host 등록, 현재 인증, epoch/scope, grant clock/expiry를 확인. TCP 연결 또는 장비 실제 준비의 증거는 아님 |
| 시작 전 확인 | 현재 설정의 start_conditions를 domain 평가기로 계산 |
| 운전 중 유지 | maintained_conditions를 같은 평가기로 계산 |
| 작업별 진입 조건 | 각 StepBinding의 conditions. 전체 작업 계획·예산·자원 획득을 사전 승인하는 값은 아님 |
| 관측 근거 | 마지막 값·취득 시각·조회 시점 age·최대 age·품질·오차·등록/관측 세대·evidence ID |

사용할 수 있는 false 관측과 사용할 수 없는 관측을 구별한다. false가 조건의 기대값이면 해당 조건은 PASS일 수 있다. 관측의 유효성만 보고 성공/실패나 장비 준비를 추론하지 않는다.

Source의 사용 가능 여부와 각 조건의 verdict는 기존 domain `Condition::evaluate`를 사용한다. 진단 사유는 값 없음, 세대·출처·형식·단위 불일치, 품질/시계/오차/age 문제를 설명한다. source는 사용 가능하지만 조건식의 기대 형식과 맞지 않는 UNKNOWN은 별도로 안내한다.

조건 평가용 Fact 구성도 공통화했다. 저장 당시의 maximum age를 현재 정책보다 우선하지 않으며, source Host와 취득 오차가 현재 FactSpec과 맞지 않으면 quality를 사용 불가로 취급한다. 이 규칙은 진단과 실제 admission 평가에 함께 적용한다. 기록 자체는 수정하지 않는다.

## 표시의 유효시간

서버는 조회 시점 기준 `display_valid_for_ns`를 함께 준다. 현재 상한은 3초이며, 사용할 수 있는 source 근거나 유효한 Host grant의 남은 시간이 더 짧으면 그 값으로 줄인다. 이것은 화면의 판정을 유지할 수 있는 보수적 표시 한계다. 운전 permit TTL이나 물리 반응시간이 아니다.

브라우저는 응답을 받은 시각부터 새로 세지 않고 **요청을 시작한 monotonic 시각 + 서버의 남은 시간**을 기준으로 삼는다. 따라서 네트워크·응답 대기가 표시 유효시간을 늘리지 않는다. 한계가 지나거나 조회가 실패하면 조건·관측·Host 권한 badge를 ‘재조회 필요’로 바꾸고 이전 판정을 별도로 남긴다.

관측의 age와 값은 ‘조회 시점’과 ‘마지막 관측값’으로 표시한다. 브라우저가 새로운 native 관측이나 현재 장비 시간을 만들어내지 않는다. 화면 갱신은 브라우저 스케줄러에 의존하며 실시간 보호 기능으로 사용하지 않는다. 모든 실제 쓰기는 기존 서버 검사를 다시 통과해야 한다.

## 화면과 조작

기존 운영 공간에 ‘운전 조건’ 메뉴와 운영 화면의 이동 버튼을 추가했다. 이 페이지의 조작은 조회와 근거 펼치기다. 기존 ‘새 실행 준비’는 기록 준비 기능이며 이 페이지의 PASS를 이용해 시작 기능으로 확장하지 않았다. 개입 ACK·동일 요청 회수·검토한 revision 유지 흐름도 그대로 검증한다.

출처의 사용자용 명칭이 아직 등록되지 않아 source/Host/step의 구성 ID를 표시한다. 실제 ‘문 닫힘’, ‘척 잠김’ 같은 의미를 임의로 붙이지 않는다. 향후 장비·공정 패키지의 이름·설명과 연결한다.

## 검증 범위

- application은 사용 가능한 false와 UNKNOWN, 관측 만료, 출처 세대 변화, 조건식 형식 불일치 및 조회의 비승격을 확인한다. 현재 source identity/uncertainty 정책의 공통 적용도 시험한다.
- HTTP 시험은 다른 셀 비노출, 미등록 관측의 UNKNOWN과 현재 계정 철회 후 조회 거부를 확인한다.
- 브라우저의 관측 없음은 실제 로컬 P 서비스 응답으로 검증한다. PASS/FAIL/만료는 application 시험이 생성한 실제 read-model JSON을 브라우저 응답 경계에 제공해 표시를 확인한다. 실행 중인 서비스에 장비 관측이나 qualification을 주입한 시험이 아니다.
- 응답 지연은 표시 한계를 늘리지 않아야 한다. desktop/mobile, JavaScript 오류, 기존 로그인·요청 응답 유실/회수·개입 ACK·구성 조회를 함께 확인한다.

실제 ROS/PLC 신호와 이 화면을 연결한 현장 인수, 실제 ROS/controller 등 하위 프로세스 상태 통합, 전체 조건 계약 wire 매핑, 설명 번역·장비용 표시명, 대규모 조회의 paging/SSE 및 복구 조작 화면은 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.

플랫폼이 관리하는 연결/reader/dispatcher의 현재 진단은 [Host 서비스 상태](../rx-runtime/HOST_SERVICE_HEALTH.md)로 연결했다. 등록된 Host 권한 문맥과 서비스 활동은 별도로 표시한다.
