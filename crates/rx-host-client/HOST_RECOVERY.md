# Host 복구 통신 연결

이 모듈은 같은 Host가 유지된 상태에서 P가 같은 저장소로 재시작한 뒤, 관리자에게 검토받은 기존 작업의 기록과 결과를 조회한다. 운전용 HostRegistration·grant·자격·Arm·새 작업 허가를 복원하지 않는다. 첫 물리 셀은 NOT_COMMISSIONED다.

## 등록 근거와 현재 확인

최초 정상 pinned 연결은 정확한 TLS 공개 인증서/배포/계약 pin, 실제 Host process configuration, source snapshot, P configuration과 producer 및 최종 등록 receipt를 원자적으로 보관한다. [불변 baseline](../rx-application/HOST_BINDING_BASELINE.md)은 당시 연결을 입증하는 역사적 자료다. 현재 연결의 권한을 그 원문으로 대신하지 않는다. 이전 unpinned 등록의 과거 근거를 사후 합성하지 않는다.

제품 daemon은 배포 설정에서만 Host별 transport registry를 만들고 현재 Runtime에 선언을 등록한다. 실제 네트워크 확인은 별도로 수행한다. UI나 HTTP body가 URI·인증서·장치 snapshot을 제공할 수 없다.

`recovery::Worker`의 Host별 mutex는 같은 worker의 중복 진행을 직렬화한다. 최종 권한과 경쟁 검사는 application writer가 담당한다. private `RestrictedHost`는 연결·구성/관측 읽기·Fence·기존 receipt·기존 native 결과 조회만 제공한다. `Deref`나 raw HostClient 접근자를 제공하지 않는다.

## 관리자와 실행 순서

1. 등록 단말의 현재 ReleaseManager가 전체 Host 셀과 runtime origin, 기존 등록/작업/미해결 제한을 조회한다.
2. 서버가 반환한 context digest와 셀 revision 집합으로 제안한다. worker가 실제 pinned read를 수집하고 writer가 동일 범위를 재검사한다.
3. 검토한 제안 digest와 revision으로 승인한다. 현재 세션·역할·단말·전체 셀 권한을 다시 확인한다.
4. writer가 같은 Fence 요청 ID/body의 전송 진입을 먼저 기록하고 worker가 보낸다. 정확히 하나의 기존 runtime Fence가 있으면 원래 outbox를 사용한다. 모호한 후보를 임의로 고르지 않는다.
5. ACK는 원래 Fence에 보관한다. 늦거나 현재 조건과 맞지 않는 응답도 사실로 남기되 자동 승격하지 않는다. 후속 실제 관측이 일치해야 RECOVERY_ONLY다.
6. 승인 범위의 기존 operation만 receipt와 원래 invocation으로 조회한다. 읽은 native evidence 중 선택 operation의 prefix를 응답에 담아도 완전한 스트림이라고 주장하지 않는다. 정상 Host publisher가 전체 원장 순서를 전달하고 P가 원래 증거 수용 규칙으로 판단한다.

제안·승인의 응답이 유실되면 같은 request key/body를 회수한다. 과거 제안의 session/terminal과 현재 값이 달라도 같은 principal의 현재 권한을 새로 확인한다. progress는 기존 승인과 기존 task를 이어 처리하며 새 승인을 만들지 않는다. 네트워크 오류만으로 새 grant 또는 장비 동작을 전송하지 않는다.

## 시간과 진단

구성/관측 read window와 현재 age의 100ms 검사는 유지한다. TLS custom connector는 tonic 기본 connector를 우회하므로 TCP_NODELAY를 직접 적용한다. P ingress와 H RPC의 명시 incoming stream에도 같은 설정을 적용한다. 작은 HTTP/2/TLS record가 Nagle/delayed ACK 조합으로 지연되는 것을 방지하는 전송 설정이며 검증 기한의 연장이 아니다.

최초 bootstrap 실패는 `rx.host-connection-error.v1`에 Host·셀·단계·형식화된 오류 코드만 남긴다. Prepare 실패에는 구성 왕복 및 source capture/age의 수치가 추가된다. 원격 오류문·metadata·키·body는 기록하지 않는다. 같은 단계/원인의 반복은 시간 수치가 달라져도 다시 출력하지 않는다. 오류 상태 자체는 기존 Attention으로 유지한다.

## 시험과 남은 범위

core 반례, 실제 terminal TLS/API routing 시험, 실제 두 이미지의 P-only restart/API/브라우저 시험, 실제 P와 별도 native fault fixture의 known-operation 조회 시험을 구분한다. 결과와 정확한 실행 소스·이미지는 문서 저장소의 단계별 evidence에 연결한다. 시험 코드가 존재한다는 사실을 통과 기록으로 표현하지 않는다.

Host 자체 재시작·source generation 변경·새 설치/store generation·공유 Host 다중 셀 실측·운전용 rebind·자격 재발급·명시 Run 재개는 이 RecoveryOnly 경계의 완료 주장이 아니다. 결과 조회 성공도 물리적 지지 인계나 자원 자동 해제의 증거를 대신하지 않는다.

Native 성공과 전체 작업 성공은 구분한다. 원래 completion에 후조건이 있고 재시작으로 permit/cell 연속성이 끊겼다면, 조회로 원래 capture를 회수하고 전체 evidence prefix를 수용해도 Work는 UNKNOWN/NONE을 유지한다. 현재의 true 관측을 과거 작업의 완료 근거로 소급하지 않는다. [실제 인수 도구](../../tools/HOST_RECOVERY_TEST.md)는 이 경우와 재전송 부재를 함께 확인한다.
