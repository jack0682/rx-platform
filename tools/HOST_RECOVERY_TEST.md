# Host 복구 연결의 인수 시험

모든 경로는 자체 생성한 FILE_SIMULATION 설정·계정·인증서·서명 자료와 disposable container/volume/network를 사용한다. 제품 DB의 writer는 제품 프로세스뿐이다. oracle는 별도 network-none 프로세스의 읽기 전용 SQLite transaction이다. 실물 장비와 사용자 운영 저장소를 연결하지 않는다.

## 유휴 Host의 복구 API

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_recovery.py --evidence-dir NEW_DIRECTORY
```

실제 두 이미지의 P/H를 초기화하고 최초 pinned 연결의 immutable baseline을 확인한다. H를 유지한 채 P만 같은 DB로 재시작한다. 새 producer/session 협상과 원래 registration 보존 후 ReleaseManager가 context→proposal→approve를 수행한다. proposal은 아직 Fence를 전송하지 않는다. approval은 현재 runtime origin의 원래 Fence ID/body를 사용한다. 같은 요청 회수, 잘못된 operation 거부, recovery-only progress와 grant/등록/baseline/제한/effects 불변을 검증한다.

## 실제 운영 화면

```sh
CARGO_INCREMENTAL=0 ../.tools/browser-python/bin/python tools/host_recovery_browser.py --evidence-dir NEW_DIRECTORY
```

같은 fixture의 별도 새 실행을 사용한다. 실제 POST가 commit됐음을 독립 API로 읽은 뒤 브라우저 응답만 유실시킨다. 새로고침 후 송신 전 저장한 동일 raw body/key로 정확히 하나의 제안을 회수한다. 승인·기록 목록 재발견·desktop/mobile·CSP·console·외부 resource와 native effects를 확인한다. 이미 API 시험이 제안한 fixture를 재사용하지 않는다.

브라우저는 disposable test CA의 trust만 우회한다. 별도의 Python terminal client는 CA와 server certificate를 검증하고, 실제 서비스는 terminal mTLS·현재 사용자·등록 단말·Origin/CSRF를 그대로 적용한다. HAR·cookie·private key·로그인 본문은 evidence로 반출하지 않는다. 자동화 접근성 선택자 실패와 제품 실패를 구분한다.

## 원래 미확정 작업의 결과 조회

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_recovery_known.py \
  --host-fixture ../.tools/linux-executor-build/debug/rx-host-recovery-fixture \
  --release-evidence EXACT_IMAGE_PREFLIGHT_JSON \
  --evidence-dir NEW_DIRECTORY
```

P/E는 실제 제품 실행파일이다. H는 S의 같은 service/RPC/publisher/FileDevice를 쓰는 test-harness 전용 `rx-host-recovery-fixture run CONFIG`다. Linux ELF/architecture/hash를 검증하고 read-only로 마운트한다. 이 실행파일을 출하 rx-hostd 자체의 fault injection 시험이라고 표현하지 않는다. 실제 rx-hostd init과 signed commissioning/API를 재사용하며 Engine 행·권한·Run·receipt를 주입하지 않는다.

- NativeAdapter는 원래 FileDevice의 효과를 저장한 뒤 결과 반환을 감춘다. 원래 SEND_ENTERED와 invocation을 유지하며 SUBMIT/effects는 각각1이다.
- P만 재시작한다. 기존 operation은 UNKNOWN/NONE/QUARANTINED이며 현재 RuntimeRestart origin이 승인 범위를 명시한다.
- 승인 후 marker가 없을 때 원래 lookup은 HIDDEN/증거0이다. marker는 모의 결과의 관측 가능 시점만 바꾸는 빈 파일이다.
- marker 생성 후 같은 operation의 원래 lookup이 FOUND를 반환한다. 별도 calls 원장과 FileDevice 효과 원장을 대조하고, 원래 capture와 실제 Host evidence/publisher prefix의 P 반영을 확인한다.
- 이 공정의 completion에는 후조건이 있다. 현재 셀과 과거 permit의 epoch/범위 연속성이 끊겼으므로 native 성공을 얻어도 전체 작업은 UNKNOWN/NONE과 QUARANTINED를 유지한다. 결과 회수와 공정 완료 판단을 구별한다.
- 반복 조회는 같은 RESULT_CAPTURED receipt/prefix를 반환한다. Host는 이미 저장한 결과를 읽으며 native lookup·SUBMIT·효과·Host evidence를 추가하지 않는다. 새 grant/자격/Arm/part 완료/Run 재개가 없어야 한다.

`--release-evidence`는 선택한 S image ID, 원문 archive/hash와 지정된 기존 recovery 시험3개의 성공 log/hash를 요구한다. 오래된 이미지 증거를 현재 이미지에 임의 적용하지 않는다. fixture build/실제 인수는 각각 기록하고 구현되지 않은 실제 장비·Host 교체·운전 rebind·재개를 통과로 보고하지 않는다.
