# S 운영 앱의 직접 단말 HTTPS 제공

P는 S가 빌드한 UI의 고정 바이트를 제공한다. UI 소스·빌드와 `/opt/rx/operator` 산출물은 S 소유이며, P에는 ROS·Node·앱 빌드 의존성을 추가하지 않는다. 제품은 기존 두 이미지를 유지한다.

P startup의 선택 `operator_ui` 설정은 `directory` 절대 경로와 `manifest: {path, sha256}`를 받는다. manifest 경로는 정확히 `directory/operator-bundle.json`이어야 한다. 설치 도구는 digest로 고정한 S 이미지에서 bundle을 꺼내 버전별로 게시하고 P에 읽기 전용으로 mount한다. P의 data/runtime/package import 경로와 겹치지 않아야 한다. 설정을 생략하면 기존 API 전용 동작을 유지하며, 설정한 bundle의 검증 실패는 기동 실패다.

manifest schema는 `rx.operator-ui-bundle.v1`, `api_schema`는 `rx.operator-api.v1`이다. `files`는 경로 오름차순 배열이며 각 항목은 `path`, SHA-256 `sha256`, Counter 문자열 `size_bytes`를 가진다. manifest 자신은 목록에서 제외한다. `index.html`과 `assets/` 아래 js/css/woff2/woff/png/jpg/jpeg/svg/ico/webp/json만 허용한다. MIME은 P의 고정 표에서 결정한다.

최대 1,024파일, 파일당 8 MiB, 전체 64 MiB다. 0바이트 asset은 거부하며 manifest는 최대 1 MiB다. 파일 경로는 공유 PackagePath의 ASCII 240자·구성요소당 100자·예약 파일명/끝점 금지 규칙과 최대 16개 구성요소를 따른다. 중복·역순·traversal·symlink·미목록 파일·특수 파일·크기/hash 불일치를 거부한다. root와 그 조상도 symlink 없이 정규화된 절대 경로여야 한다.

manifest/asset 바이트는 기존 `rx-package::directory::read_relative_file`로 root 아래에서 취득한다. 중간 디렉터리/leaf nofollow, Unix NONBLOCK, 실제 열린 descriptor의 regular-file 검사와 읽기 상한을 재사용한다. 파일 목록 검사는 이후 open의 권한 증거가 아니며, 취득 전에 FIFO·링크로 교체되더라도 일반 파일로 읽지 않는다. 검증한 바이트를 메모리에 보유하므로 요청 시 디스크를 다시 열지 않는다. 실행 중 bundle 변경은 지원하지 않으며 다음 기동에서 재검증한다. 이 보강은 신뢰하는 배포 root 아래 파일 취득의 범위다. 읽기 전용 배포 root·신뢰하는 host OS 전제는 유지하며 악성 host 전체의 경로/마운트 교체를 방어한다고 주장하지 않는다.

UI와 data/runtime/import의 중첩 비교는 기존 ancestor를 실제 경로로 해석한 뒤 아직 생성되지 않은 디렉터리 suffix를 붙여 수행한다. UI 설정이 있으면 비교 대상의 `..`는 명시적으로 거부하고, dangling link·디렉터리가 아닌 ancestor도 거부한다. `/var`처럼 이미 있는 경로 별칭은 실제 위치가 분리되어 있으면 허용한다. 비교 자체가 디렉터리를 만들지는 않는다.

`TerminalHttps::new_with_operator_ui`가 정적 router와 기존 API router를 합친다. 이전 생성자들은 bundle 없는 wrapper로 유지한다. TLS가 검증한 실제 client leaf만 TerminalPeer를 만든다. 정적 GET/HEAD도 같은 mTLS·Host/origin 아래 제공되지만 사람 session을 생성하거나 업무 권한을 부여하지 않는다. 미등록 단말의 로그인 거부, 등록 단말·사용자 session·역할 검사는 기존 `/api`가 유지한다. cookie는 기존 `Path=/api; Secure; HttpOnly; SameSite=Strict`다. 프록시 인증서 헤더는 신뢰하지 않는다.

정적 route는 `/`, `/index.html`과 manifest에 있는 정확한 asset 경로뿐이다. SPA fallback이나 임의 디렉터리 제공은 없다. 존재하지 않는 API·asset·deep link는 기존 JSON 404이며, 정적 경로의 POST는 405다. API는 기존 CSP를 유지하고 UI는 script/style/font/connect/img를 self로 제한한다. inline script/style, eval, 외부 CDN은 허용하지 않는다. 응답은 no-store/nosniff이며 HEAD는 본문 없이 같은 Content-Length를 제공한다.

검증 대상은 owned byte 보존, manifest/asset 변조·크기/경로 반례, 실제 mTLS 정적/API 조합, 단말 cookie 재생·폐기 및 fresh startup의 무운전 상태다. 실제 S production bundle과 브라우저의 결합 검증은 별도로 수행한다. UI 접속이 qualification이나 Run 시작을 뜻하지 않는다.

브라우저/이미지 시험 입력은 기존 `rx-platformd` composition의 ignored `export_container_fixture`로 내보낸다. `RX_PLATFORM_IMAGE_FIXTURE`는 새 출력 디렉터리, `RX_PLATFORM_IMAGE_PORT`는 외부 HTTPS 포트다. 선택 `RX_PLATFORM_OPERATOR_BUNDLE`에 실제 S bundle 경로를 주면 원본을 검증하고 startup은 `/operator` read-only mount를 참조한다. 기존 `/config`·`/data` mount를 사용한다. `terminal.pem/key`는 catalog에 등록된 시험 단말이며 `browser-fixture.json`에 시험 로그인 정보가 있다. 기존 `probe.pem/key`는 서비스 peer이므로 사람 단말용으로 바꾸어 쓰지 않는다.
