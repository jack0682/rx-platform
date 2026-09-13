<!-- RX-HARNESS:BEGIN -->
## RX project harness

프로젝트는 rx-ws다. 사용자의 현재 요청과 이 저장소의 지침을 우선한다.
선택적 로컬 작업 프로파일: `../../my_harness/projects/rx-ws/PROJECT.md` (해당 설치가 있을 때만 읽는다).
제품 목표의 공개 원본: https://github.com/jack0682/rx_docs/blob/main/docs/01_product_definition.md .
로컬 workspace가 있으면 `../AGENTS.md`도 읽는다. 단독 clone은 이 저장소 지침으로 작업한다.
로컬 하네스 설치가 있을 때 복잡한 작업에는 `../../my_harness/codex-harness/KERNEL.md`, `../../my_harness/codex-harness/ROUTING.md`와 프로파일의 context·constraints·local_rules를 읽는다.
문맥 확인: `python3 ../../my_harness/tools/rx_harness.py context --agent codex`.
세션은 이 프로젝트·에이전트의 기록만 사용하고 현재 요청·대상과 일치할 때 재개한다.
기존 rx_poc의 shared 상태·작업 큐·전역 최신 세션을 자동으로 이어받지 않는다.
직접 작업에서는 사용자에게 맡은 범위의 계획·구현·검증을 수행한다.
여러 에이전트가 함께 쓰면 파일 소유를 나누고 실제 검증 근거를 남긴다.
legacy tmux co-work는 명시적으로 참여한 작업에만 적용하며 그 상태를 수정하거나 PASS를 만들지 않는다.
이 연결은 도구 권한·모델·장비 운전 권한을 변경하지 않는다.
생성 원본: `../../my_harness/tools/rx_harness.py install`. 이 표시 영역 밖의 기존 지침은 보존한다.
<!-- RX-HARNESS:END -->

# Repository instructions

- RX is a public personal project for heterogeneous robots and infrastructure. This repository owns ROS-independent Rust authority, contracts, records and APIs. Project contributions use Apache-2.0; preserve third-party notices.
- Read README.md and CONTRIBUTING.md. The repository builds independently; sibling checkouts and local agent harnesses are optional.
- Work from develop on feature branches. Keep main stable; use PRs and required CI checks. Follow repository-settings.json for GitHub administration.
- Preserve task identity, execution authority, uncertainty, evidence provenance and resource-handover semantics. Simulation success does not establish physical qualification.
- Record contract changes and compatibility impact. Shared SDK changes originate in rx-platform and must be regenerated and checked.
- Run the relevant Rust/UI/protocol tests and repository checker. Report exact exercised scope and remaining limitations.
- Keep credentials, local toolchains, machine-specific agent settings and runtime data out of commits. Preserve unrelated user changes.
