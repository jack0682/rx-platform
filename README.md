# RX Platform

Rust 기반 RX core·application·저장·전송·설치복원 도구. 전체 설계와 규범 원본은 [rx_docs](https://github.com/jack0682/rx_docs)에서 관리하며, 빌드·검증용 규범 사본은 이 저장소의 `spec/`에 고정한다.

`rx-domain`은 순수 계약 의미와 판단을 소유한다. ROS·BT·SQLite·HTTP 의존을 두지 않는다. 실제 I/O와 저장 원자성은 바깥 adapter가 구현한다.

현재는 구현 중이다. 지원 완료·현장 운전 가능 상태를 의미하지 않는다. 전체 목표와 상태는 [구현 기록](https://github.com/jack0682/rx_docs/tree/codex/initial-draft/docs/implementation)을 따른다. 장비·ROS·공정·운영 앱은 [rx-solutions](https://github.com/jack0682/rx-solutions)에 둔다.

로컬 실행 입구와 인증/HTTP 경계는 [rx-api](crates/rx-api/README.md)에 정리했다. `rx-platform-local`은 실제 writer/SQLite를 사용하는 개발 서비스이며 Host/장비 launcher가 없는 loopback 전용이다.

공유 SDK를 사용하는 두 레포를 함께 검증할 때는 `python3 tools/check_host_sdk.py ../rx-solutions/sdk`로 현재 platform export와의 일치도 확인한다. SDK의 자체 source-lock만 일치하는 오래된 사본을 통과시키지 않는다.
