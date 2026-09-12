# Host 실행 서비스의 현재 상태 진단

2026-09-11. 플랫폼이 관리하는 Host 연결·조회·전달 서비스의 상태를 운영 화면에 연결했다. 이 정보는 **진단용 메모리 상태**이며 qualification, Host grant, dispatch permit, 작업 결과의 입력으로 사용하지 않는다.

## 소유와 수명

`rx-runtime::Application`의 전용 registry가 writer 안에서 상태를 관리한다. SQLite·제어 원장에 heartbeat를 계속 추가하지 않는다. Application 재생성 시 registry는 비어 있으며 이전 실행의 정상 표시를 복원하지 않는다.

로컬 supervisor가 처음에 구성 대상(host/cell)을 등록한다. 전체 대상의 유효성·중복·최대 64개를 확인한 뒤 적용하므로 잘못된 뒤쪽 항목 때문에 앞쪽만 구성되는 일이 없다. 초기화하지 않은 환경은 진단 미연결, 초기화했지만 대상이 없는 경우는 미구성, 대상은 있으나 아직 보고가 없는 경우는 첫 보고 대기다.

각 대상은 runtime boot·cell definition/envelope에 묶인 opaque owner ID를 받는다. owner ID는 브라우저에 노출하지 않는다. 보고는 해당 owner와 증가하는 sequence에 맞아야 한다. 오래된 owner, 중복/후퇴 sequence, 다른 clock 또는 미래 활동 시각을 거부한다. 표시할 때 현재 definition/envelope와 달라졌으면 구성 불일치로 처리한다.

내부 `ReplaceHostService` 명령은 **진단 보고 owner만** 교체한다. 실제 프로세스나 장비를 재시작하지 않는다. 기존 보고를 버리고 새 owner의 첫 보고를 기다리며, 옛 작업자의 뒤늦은 보고로 덮어쓸 수 없다. 실제 restart/rebind 권한은 별도 절차다.

## 세 가지 서비스 상태

| 항목 | 값의 출처 | 해석 |
|---|---|---|
| 연결·사용권 조정 | ConnectionService의 Waiting/Connecting/Bound/Attention/Stopped | Bound는 연결 등록 경로가 완료됐다는 상태. 매 순간 TCP가 살아 있다는 증거가 아님 |
| 관측 조회 | ObservationReader의 결과와 P 수신 시각 | 응답 수신과 값의 유효성은 구별. 연속성 문제·조회 불가·대기를 따로 표시 |
| 명령 전달 루프 | Dispatcher가 한 번의 유한 처리 pass를 마친 뒤 얻은 P 시각 | 루프의 최근 활동. 명령 성공·native 완료·queue 비움의 증거가 아님 |

전달 오류가 한 번이라도 기록됐으면 오류 이력을 별도로 표시한다. 그 이력 자체를 현재 작업 실패로 바꾸지 않는다. 원시 오류 문자열, 전달/재조정 횟수와 다른 셀의 operation ID를 UI로 보내지 않는다. Dispatcher는 Host 범위의 서비스이므로 그 활동은 특정 셀의 완료 수량이 아니다.

## 보고와 활동의 시간은 다르다

relay는 기본 500 ms와 연결 상태 변경 시 최신 상태를 전달한다. 빠른 observation 변경마다 별도 진단 메시지를 쌓지 않는다. 대기열이 바쁘면 최신 상태를 다음에 다시 전달하며, 누락된 보고를 정상 값으로 채우지 않는다. 조회 실패가 있어도 마지막 실제 수신 시각은 보존한다.

보고 자체의 현재성 한계는 3초다. 보고가 멈추면 STALE이며 정상 상태로 표시하지 않는다. **relay heartbeat가 새로 와도 마지막 관측 수신·전달 pass 시각은 늘어나지 않는다.** 최근 보고는 있지만 worker 활동이 오래됐다면 ‘최근 관측 응답/루프 동작 확인 필요’로 보여준다. 이 3초는 현재 진단 표시 정책이며 물리 보호·실시간 성능 기준이 아니다.

registry는 API overview의 현재 접근권으로 걸러진 셀에만 진단을 붙인다. 같은 writer에서 읽기와 decoration을 순서대로 수행한다. UI 표시 수명도 유효한 보고/활동의 남은 시간보다 길게 만들지 않으며, 기존 요청 시작 기준의 보수적 만료 계산을 따른다.

## 실제 기동과 종료

`rx-platformd`는 Host 서비스 구성과 owner 할당을 API 서비스 시작 전에 수행한다. 각 ConnectionService의 실제 watch 채널을 relay에 연결한다. relay 생성 시 owner의 host/cell과 service 대상이 같은지 확인한다. 다른 서비스의 활동을 잘못된 owner로 보고하는 연결을 거부한다.

producer가 아직 없으면 Host 인증 대기로 표시한다. 등록된 사용권이나 last-known source 값을 근거로 임의의 성공 상태를 만들지 않는다. 연결 서비스의 dispatcher report와 observation report를 직접 읽으며, adapter/컨트롤러의 전체 내부 상태를 추측하지 않는다.

진단 relay는 핵심 권한 서비스와 별도 task 집합으로 종료한다. 종료 시 마지막 Stopped 보고를 시도하고 최대 2초 안에 회수한다. 진단 전달 실패를 이유로 native 작업을 재제출하거나 허가를 변경하지 않는다. 권한 철회·dispatcher drain·물리 정지 확인은 기존 경로를 따른다.

## UI·호환·검증

`HostDiagnostic.runtime`에 optional `rx.host-service-health.v1` snapshot을 추가했다. 이전 진단 모델의 값·조건 판정은 그대로 유지하며, 진단이 없으면 미연결을 표시한다. base/cell 규범8개와 optional wire binding4개, SDK79개는 변경하지 않았다.

검증은 owner 교체/늦은 보고·sequence 후퇴 거부, 보고 만료, heartbeat와 활동 시간 분리, 현재 사용자 범위·권한 철회, 새 Runtime의 이전 owner 거부, 부분 inventory 방지와 실제 P 기동 경로의 인증 대기 보고를 포함한다. Host TLS 통합에서는 실제 dispatcher pass 시각도 확인한다.

브라우저의 서비스 상태 분기는 HTTP 시험이 P에서 생성한 snapshot을 응답 경계에 제공해 확인한다. 이를 실장비 상태 검증으로 세지 않는다. 기존 조건·요청 복구·개입 ACK·모바일 흐름도 유지한다.

아직 실제 ROS/controller/GPU 프로세스 health, 현장 중단 반응시간, 자동 Host 프로세스 배치·재시작과 전체 설치/복원 supervisor를 구현·인수한 것은 아니다. 로컬 상태 파일의 전체 서비스 요약과 operator용 지원 로그 조회도 후속이다. 첫 물리 셀은 NOT_COMMISSIONED다.
