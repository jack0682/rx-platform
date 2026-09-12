# 사용자·등록 단말·HTTPS 신원 연결

2026-09-11. 사람의 사용자 세션과 실제 TLS 연결에서 확인한 등록 단말을 결합했다. 단말 인증서·사용자 계정·운전 조건은 서로 다른 확인 항목이다. 인증 성공만으로 qualification, 진입 조건, Arm 응답, native 결과를 만들지 않는다.

## 처리 경계

```mermaid
flowchart LR
    Browser[단말의 사용자] --> TLS[TLS 클라이언트 인증서 검증]
    TLS --> Password[제한된 비밀번호 검증 worker]
    Password --> Writer[현재 사용자와 단말 등록 확인]
    Writer --> Session[단말 등록 버전에 묶인 사용자 세션]
    Session --> Command[현재 신원·역할·셀 범위 검사]
    Command --> Authority[기존 조건·사건·예산·허가 처리]
```

`rx-api::terminal_https::TerminalHttps`는 API ingress 구성 요소다. 기존 단일 Runtime writer를 받아 같은 application command를 호출한다. ROS·장비 SDK나 driver를 기동하지 않는다. 실제 listener·TLS 자료·프로세스 수명주기를 제공하는 제품 supervisor/installer 연결은 후속이다. 개발 CLI를 LAN 서비스로 바꾸지 않았다.

## 신원 발급

1. 서버는 설정된 CA에 대한 client certificate를 요구한다. 인증서 없이 또는 다른 CA의 인증서로 연결하면 HTTP handler까지 도달하지 않는다.
2. TLS가 제공한 leaf DER의 SHA-256으로 단말을 식별한다. `x-forwarded-client-cert`, `x-rx-terminal`, body의 단말 이름을 증거로 받지 않는다.
3. 기존 bounded Argon2 worker가 비밀번호를 확인한다. Root CA가 유효해도 등록되지 않은 단말은 사용자 세션을 받지 않는다.
4. writer는 현재 활성 사용자와, 인증서에 정확히 대응하는 활성 Terminal 하나를 확인한다. 사용자와 단말의 허용 셀에 겹침이 있어야 한다.
5. Session에 `TerminalBinding{id, certificate_digest, revision}`을 저장한다. 요청의 Identity는 이 결과에서 만들며 body가 결정하지 않는다.
6. 브라우저에는 기존 256-bit opaque token cookie를 발급한다. token 자체는 권한 자료가 아니며 서버 메모리에는 digest만 저장한다. HTTPS cookie에는 Secure, HttpOnly, SameSite=Strict, `/api` 경로를 지정한다.

일반 개발 로그인 세션은 terminal=None이다. 내부에서 그런 세션의 Identity에 단말 ID/인증서만 덧붙여도 코어가 거부한다. Host/Executor/OPERATOR_API 계정은 사용자 로그인 세션으로 열 수 없다.

## 매 요청의 확인

HTTPS middleware는 cookie의 단말 인증서와 현재 TLS 연결의 인증서를 비교한다. 따라서 동일한 CA가 발급한 다른 등록 단말에서도 cookie를 그대로 사용할 수 없다. 같은 단말의 정상 TLS 재접속은 가능하다.

writer는 Session의 runtime boot·만료·활성, 현재 사용자 역할, 저장된 TerminalBinding과 현재 단말 등록 revision·활성·인증서를 다시 확인한다. 유효 셀은 사용자 셀과 단말 셀의 교집합이다. overview/profile/개별 조회에도 이 범위를 적용한다. 저장된 idempotency 응답을 반환하기 전에 현재 신원과 권한 검사를 수행한다.

단말 등록에 실제 변경이 생기면 이전 세션을 더 이상 사용할 수 없고, 관련 셀 closure의 권한을 철회·latch한다. pending start도 무효화된다. 같은 내용의 등록 요청은 정확한 CAS 아래 기존 revision을 반환하므로 단순 재적용으로 세션을 끊지 않는다. 등록 변경과 그에 필요한 invalidation은 같은 writer transaction이다.

로그아웃은 해당 사용자 세션을 종료한다. 이미 접수된 운전 의도를 취소하거나 물리 동작을 정지하는 명령과 동일하지 않다. 네트워크 단절/HTTP 응답 상실도 기존 명령을 rollback하지 않는다. 원래 key/body의 회수와 현재 P 상태 확인 규칙을 유지한다.

만료되었거나 서버 재시작으로 사라진 cookie가 있어도 login 자체는 새 비밀번호·현재 TLS 단말로 다시 시도할 수 있다. 로그인 요청이 아닌 경로에서는 오래된 cookie를 계속 사용하지 못한다.

## 시작 요청에서 확인한 동작

실제 HTTPS 시험은 synthetic qualification/Host 준비를 가진 모의 셀에서 Alice + 등록 panel/a가 StartRun을 요청하면 ARMING까지 접수됨을 확인했다. Host Arm 응답은 만들지 않았으므로 EXECUTING이나 native 성공을 주장하지 않는다.

동일 시작 key 재전송은 같은 attempt를 회수한다. 단말을 비활성화하면 현재 cookie와 같은 key를 다시 보내도 거부되고 pending start의 기존 권한이 복원되지 않는다. qualification이 없는 셀은 정상 사용자·정상 단말이어도 NOT_COMMISSIONED로 거부한다. 사용자 역할 철회도 cached 요청보다 먼저 검사한다.

## 사람과 서비스의 분리

서비스 계정에 Operator/RecoveryLead 역할을 추가하여 사람처럼 ProcedureRecord를 보내거나 case lead가 되는 경로도 차단했다. 서비스의 native/관측 증거는 Host evidence 경로로 수용하며 사람의 절차 보고로 둔갑시키지 않는다. 기존 저장 기록은 삭제하지 않는다.

이로 인해 이전 초안의 ‘executor 인증 + 추가 사람 역할’ 방식은 인간 보고 경로로 더 이상 사용하지 않는다. frozen RecordProcedure 등의 gRPC 인간 쓰기는 검증된 사용자·단말 세션을 OPERATOR_API 호출과 결합하는 별도 binding을 연결한 후 활성화해야 한다. 현재 직접 HTTPS 및 개발 HTTP의 실제 사용자 경로가 application을 호출한다.

인증은 인증서 개인키의 소지와 계정 자격을 확인한다. 사람의 물리적 현장 위치·주의·안전기능 상태를 입증하지 않는다. 물리적 접근/격리·복구 guard와 실제 작업 결과는 각 절차·Host/장비 근거로 따로 검증해야 한다.

## 서버 구성과 한계

| 항목 | 현재 구성 |
|---|---|
| 모듈 | rx-api `TerminalHttps::new` / `serve` |
| 입력 | 기존 ApplicationPort, Credentials, 정확한 HTTPS origin, server certificate/key, terminal CA |
| origin | 직접 same-origin HTTPS; 프록시의 인증서 header를 신뢰하지 않음 |
| HTTP | HTTP/1.1, 최대 message body 1 MiB, login body 4096 bytes |
| 동시 연결/task | 최대 64 |
| TLS/header 대기 | 각각 최대 5초 |
| HTTP 연결 수명 | 최대 60초; 아직 장시간 SSE/stream 경로를 제공하지 않음 |
| shutdown | 새 접속 중지, 미완성 handshake 중단, HTTP graceful drain 후 네트워크 연결 정리 |
| 인증서 재사용 | early data와 서버 session storage/TLS 1.3 ticket 발급 비활성 |
| 사용자 세션 | 기존 1시간, 현재 P session 만료 및 단말 등록 재검사 |
| TLS 자료 | 각 입력 최대 128 KiB, certificate chain 최대 16개; 이미지 내 배포/비밀 mount 관리는 후속 |

클라이언트 인증서 배포·회전/폐기 UI, 실제 브라우저 인증서 설치, S의 UI endpoint와 제품 HTTPS routing, OPERATOR_API gRPC 사용자 위임, 제품 daemon/두 이미지 배포는 완료하지 않았다. 신규 TLS 라이브러리의 서비스 수명주기 검증을 전체 장비 supervisor/현장 인수 완료로 확대하지 않는다.

API는 기존 versioned mutation 처리기를 재사용한다. 신규 HTTPS 구성은 직접 연결이 전제이며 TLS를 종료하는 reverse proxy 뒤에서 forwarded header를 단말 증거로 바꾸는 동작을 제공하지 않는다.

## 저장 버전과 SDK 동기화

Session 형식과 신원 검사의 의미가 바뀌므로 저장소는 schema5다. 이전 runtime이 terminal binding을 무시하고 새 기록을 읽지 못하도록 version barrier를 올렸다. 기존 session bytes는 그대로 두며 terminal proof를 자동으로 채우지 않는다. Runtime boot의 기존 session 무효화 규칙도 유지한다.

검토 중 phase32의 P 저장소는 schema4로 올라갔지만 S SDK 사본은 schema3에 머무른 것을 확인했다. 이번에는 공유 rx-storage와 migration 0004/0005를 함께 수출해 양쪽을 schema5로 맞췄다. SDK payload는 74개이며 별도 source-lock까지 75개 파일이다. Authority/application 코드는 SDK에 넣지 않는다.

`python3 tools/check_host_sdk.py ../rx-solutions/sdk`는 SDK 자체 inventory뿐 아니라 현재 P에서 새로 수출한 결과와 byte 단위로 비교한다. 실제 phase32 archive의 SDK를 입력하면 자체 hash가 맞아도 stale로 거부하는 회귀를 확인했다. 각 레포의 시험 통과만으로 사본 동기화를 추정하지 않는다.

## 의존성과 근거

기존 lock의 rustls 0.23.44 / tokio-rustls 0.26.5 / hyper 1.11.1 / hyper-util 0.1.20을 사용했다. 테스트용 reqwest 0.12.28의 manual-root TLS feature를 추가하면서 hyper-rustls 0.27.9 등 잠금 항목이 추가됐다. QUIC 관련 optional lock 항목의 존재를 운영 API의 HTTP/3 지원으로 해석하지 않는다. 실제 API는 HTTP/1.1만 사용하며 normal dependency tree도 저장했다.

Axum router의 Tower service를 Hyper 연결에 전달하는 adapter는 [hyper-util 공식 API](https://docs.rs/hyper-util/0.1.20/hyper_util/service/struct.TowerToHyperService.html)를 확인했다. Rustls verifier/PEM 및 Hyper timeout 구성은 사용 중인 pinned crate 원본과 실제 빌드/연결 시험으로 확인했다. 두 버전 문서 URL의 웹 조회 실패를 인증서 검증 근거로 사용하지 않았다.

## 검증 범위

- 실제 TLS: 인증서 없음/다른 CA/미등록 단말, forwarded header·body 사칭, 다른 등록 단말의 cookie replay, 오래된 cookie 이후 재로그인, Secure logout.
- 실제 writer/SQLite: 사용자·단말 scope 교집합, 등록 revision 변경과 이전 세션 거부, 잘못 조립한 Identity 거부, 사용자 역할 철회, 정상 시작 접수/같은 key 회수/단말 철회 후 재요청 거부.
- qualification 미충족 거부, 사람과 서비스 역할 혼합의 절차 보고/lead 거부, 미완성 TLS 연결을 둔 서버 shutdown.
- macOS와 격리된 Linux 환경의 플랫폼 시험, S SDK/실행기 시험, 기존 별도 Host/Executor/Evidence 및 브라우저 회귀.

실물 qualification·물리 정지·실제 CNC/로봇 통신·사용자 단말 배포 검증은 수행하지 않았다. 이 문서는 구현된 인증 경계와 검증 범위를 기록한다.
