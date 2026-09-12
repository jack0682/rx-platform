# 두 제품 이미지의 셀 납품 경로 시험

`tools/test_cell_delivery.py`는 실제 P/S runtime 이미지를 사용하여 초기 설치에서 운영 화면의 완료 표시까지 확인한다. 장비는 독립적인 파일 원장을 가진 `FILE_SIMULATION`이다. 실물 장비 주소나 device mount를 받지 않는다.

## 실행과 입력

P 저장소에서 Playwright가 설치된 Python으로 실행한다.

```sh
CARGO_INCREMENTAL=0 ../.tools/browser-python/bin/python tools/test_cell_delivery.py \
  --evidence-dir ../references/implementation/cell-delivery-new \
  --platform-image rx-platform:runtime-draft \
  --solutions-image rx-solutions:runtime-draft \
  --release-evidence ../references/implementation/phase76_checks.json
```

증거 디렉터리는 존재하지 않는 새 경로여야 한다. `--release-evidence`는 선택한 S 이미지의 정확한 digest와 봉인된 복구 시험·소스 archive를 포함해야 한다. 새 이미지를 빌드했다면 해당 소스와 시험으로 새 증거를 만들고 지정한다. 과거 이미지의 기록을 새 이미지의 검증으로 사용하지 않는다. `--prepare-only`는 자료 생성·서명·컴파일만 확인하며 runtime 인수를 주장하지 않는다.

기본 `--composition independent`는 P·Host·Executor의 세 운전 컨테이너다. `--composition supervisor`는 P 하나와 S 하나를 사용하며 S의 제품 supervisor가 status·Host·Executor 프로세스를 직접 관리한다. 준비·검사·초기화용 일회성 컨테이너는 운전 컨테이너 수와 구별한다. 두 방식 모두 제품 이미지 종류는 두 개다.

필요 도구는 Docker, repository Rust toolchain/cache, Python Playwright/browser다. 이 도구가 일반 사용자의 ROS 환경이나 장비 설정을 변경하지는 않는다. 임시 TLS 신원·서명 fixture는 매 실행 새로 만들고 종료 시 제거한다.

## 실제 경로와 판정

1. Rust fixture exporter는 비어 있는 초기 공정과 공개 정책·시험용 신원을 만든다. Engine 상태, 세션, Host ACK, 운전 자격은 주입하지 않는다.
2. S 이미지의 패키지 도구가 원본을 조립하고 외부 시험 서명을 받아 봉인·컴파일한다. 최종 설치 자료는 그 실제 결과에 결합한다.
3. 선택한 composition으로 기동한다. supervisor 방식은 실제 제품 CLI가 파일 pin과 설치 identity를 대조하고 명시적 init을 수행한 뒤 status → Host → Executor 순서로 기동한다. READY는 소프트웨어 프로세스 상태이며 별도의 운전 허가를 만들지 않는다.
4. 등록 단말의 mTLS와 서로 다른 사용자 세션으로 반입, 검토, 독립 승인, 구성 변경, Host 적용 확인, 재검증, 자격 수용, 활성화를 수행한다.
5. 모의 셀의 여섯 검증 영역은 실제 측정·확인한 assertion과 한계를 포함한다. 복구 영역은 정확한 S 릴리스의 봉인된 장애 시험을 참조한다. 이 실행 자체가 전원 상실 시험을 수행했다는 뜻은 아니다.
6. 실제 운영 화면에서 수량 2 실행을 준비하고 시작한다. 시작 응답을 의도적으로 유실시킨 뒤 원래 요청의 결과를 복구한다.
7. P의 소재 시도·작업 결과·자원 인계와 Host의 별도 파일 동작 기록을 대조한다. 두 operation ID가 일치하고 동작이 중복되지 않아야 한다. 화면에도 최신 완료 상태가 나타나야 한다.
8. 별도 PHYSICAL 셀은 `NOT_COMMISSIONED`로 남으며 실제 시작이 거부되어야 한다. operator의 구성 반입 요청도 거부되어야 한다.
9. Executor와 Host를 협력 종료한다. supervisor 방식은 관리자에 TERM을 한 번 보내고 Executor → Host → status 종료 순서를 대조한다. 각 guarded child의 실제 exit 0와 같은 instance/PID/scope/원문 digest의 STOPPED 상태가 함께 있어야 한다. 관측 기한을 넘기면 시험 실패로 남기며 정상 종료 판정을 위해 강제 종료하지 않는다. Host의 실제 safe-to-drop 결과도 보존한다. P 종료는 미설정 셀 차단 시험이나 Host fence의 잔여 attention 때문에 2일 수 있으며 전체 현장 종료 성공으로 표시하지 않는다.

SIMULATION의 permit TTL은 공개 envelope/profile에 1초로 명시한다. 조회 snapshot 수명 100ms와 별개다. 이는 이 파일 장비와 전달 경로의 시험 설정이며 실물 시간 한계의 추천값이 아니다. 실제 runtime gate와 Linux BOOTTIME을 그대로 사용한다.

## 증거와 보존 범위

`preparation.json`은 자료 준비만, `result.json`의 PASS는 위에 명시한 runtime 경로만 의미한다. API 요청·응답의 공개 자료, 변조 확인 digest, 독립 native effects, 종료 상태, desktop/mobile 화면을 보존한다. `public-materials`에는 명시적으로 선택한 공개 seed, 봉인 패키지, 컴파일 결과, 서명된 공정·재검증 보고서와 모든 참조 artifact, 공개 검증 정책을 원문과 파일별 inventory로 보존한다. 로그인 암호, session cookie, TLS private key, 서명 seed는 증거에 저장하지 않는다.

Host/Executor 네트워크는 격리한다. P의 단말 HTTPS만 loopback으로 노출하고 별도 terminal bridge에 연결한다. Python 단말 연결은 생성된 CA와 leaf를 검증한다. 시험용 브라우저는 임시 CA의 OS trust 등록을 생략하지만 별도의 단말 client certificate를 사용한다.

cleanup은 이 실행이 소유한 모의 컨테이너·네트워크·volume에 한정한다. 실패 시 cleanup을 위해 수행한 종료는 제품의 정상 종료 증거로 계산하지 않는다. 실제 장비 지원, 현장 시운전, 무인 가동 신뢰도 및 모든 복원 시나리오는 이 PASS 범위에 포함되지 않는다. supervisor 방식의 PASS 역시 해당 파일 장비 셀과 검사한 기동·종료 경로에 한정한다.
