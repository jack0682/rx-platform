# 기여 안내

RX는 이기종 로봇·시설이 공통 작업·권한·상태·결과·복구 계약으로 협업하도록 만드는 개인 프로젝트입니다. 저장소의 공개 여부와 소프트웨어의 실물 검증 수준은 별개입니다. 현재 검증 범위는 각 저장소 README와 구현 기록을 따릅니다.

## 브랜치와 GitFlow

| 브랜치 | 역할 | PR 대상 |
|---|---|---|
| `main` | 안정 기준과 릴리스 이력, 기본 브랜치 | 변경은 PR로만 반영 |
| `develop` | 다음 변경의 통합과 검증 | 준비되면 `main`으로 승격 PR |
| `feature/*`, `fix/*`, `docs/*`, `chore/*`, `codex/*` | `develop`에서 시작하는 작업 | `develop` |
| `release/*` | `develop`에서 분기한 릴리스 준비 | `main`, 필요한 수정은 `develop`에도 반영 |
| `hotfix/*` | `main`에서 분기한 긴급 수정 | `main`, `develop`에도 반영 |
| `dependabot/*` | 자동 의존성 갱신 | 일반 갱신은 `develop`, 보안 갱신은 `main`도 허용 |

작은 개인 프로젝트에서는 별도 release 브랜치 없이 `develop` → `main` 승격 PR을 사용할 수 있습니다. main 반영 후에는 `main` → `develop` PR로 변경과 이력을 다시 합칩니다. 릴리스·긴급 수정·장기 브랜치 간 병합은 **merge commit**을 사용해 공통 조상을 유지합니다. 작업 브랜치 PR은 squash merge도 가능합니다. rebase merge는 사용하지 않습니다.

```sh
git switch develop
git pull --ff-only origin develop
git switch -c feature/your-change
# 수정하고 아래 검증을 실행합니다.
git add <changed-paths>
git commit -m "Describe the behavior change"
git push -u origin feature/your-change
# GitHub에서 develop 대상 PR을 만듭니다.
```

`main`과 `develop`은 삭제·강제 push를 금지하고 PR, 최신 기준 브랜치와의 CI 성공, 모든 검토 대화 해결을 요구합니다. 단독 관리자가 자기 PR을 처리할 수 있도록 필수 타인 승인 수는 0입니다. CODEOWNERS는 검토 책임자를 지정합니다. 관리자와 자동화도 병합 전 검증 결과를 확인합니다. Ruleset의 필수 check 이름은 세 저장소 모두 `CI`입니다.

PR 경로 검사는 GitHub의 이벤트 JSON을 직접 읽습니다. 외부 fork의 `main`, `develop`, `release/*`, `hotfix/*` 이름으로 이 저장소의 릴리스 경로를 대신할 수 없습니다. 외부 기여는 작업 브랜치로 `develop`에 제안해주세요. CI는 공개 fork 코드에 저장소 비밀정보나 쓰기 권한을 제공하지 않습니다.

## 변경과 검증

PR에는 문제, 변경 후 동작, 실행한 검증, 미검증 범위를 적습니다. 기능과 무관한 대규모 정리 작업을 섞지 않습니다. 권한·불명 결과·정지·자원 인계·복구 의미를 바꾸면 반례와 호환성 영향을 함께 설명합니다.

`rust-toolchain.toml`의 Rust와 Python 3가 필요합니다.

```sh
python3 .github/test_repository.py
python3 tools/check_repository.py
python3 tools/check_contract_baselines.py
python3 tools/test_export_host_sdk.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

전체 Rust workspace와 SDK export 회귀를 검사합니다. `#[ignore]`로 표시된 다중 저장소 통합 시험, 컨테이너·실물 시험은 자동 core CI에 포함되지 않으므로 해당 변경에서 별도 실행 근거를 남깁니다.

CI는 모든 PR과 `main`, `develop`, 작업·릴리스·긴급 수정 브랜치 push에서 실행됩니다. GitHub Actions 화면에서 수동 실행할 수도 있습니다. 실패하거나 취소된 하위 검사는 종합 `CI` 성공으로 처리되지 않습니다. `tools/check_repository.py`는 Git 추적 파일과 ignore되지 않은 새 파일의 JSON 구문, 저장소 내부 Markdown 파일 링크, 규범 8개 문서의 hash와 두 manifest의 hash를 검사합니다. 외부 URL, Markdown anchor, 형제 저장소 파일의 존재는 이 검사 범위에 포함하지 않습니다.

## 저장소 간 변경

설계·규범 원본은 [rx_docs](https://github.com/jack0682/rx_docs), 플랫폼과 규범 사본은 [rx-platform](https://github.com/jack0682/rx-platform), Host·장비·서비스와 고정 SDK는 [rx-solutions](https://github.com/jack0682/rx-solutions)에 있습니다. 계약 변경은 원본의 revision·호환성·manifest를 먼저 기록하고 플랫폼 spec과 solutions SDK를 함께 동기화합니다. 세 저장소의 PR을 서로 연결하고 호환되는 커밋 조합을 남깁니다.

플랫폼의 `python3 tools/check_host_sdk.py ../rx-solutions/sdk`는 현재 플랫폼과 SDK의 일치까지 검사합니다. solutions의 독립 CI는 자체 SDK inventory를 검사하므로 최신 플랫폼과의 호환성을 대신 증명하지 않습니다. 두 저장소가 같이 바뀌면 이 별도 동기화 검사를 실행해야 합니다. SDK 사본을 직접 고치기보다 플랫폼 원본에서 재생성합니다.

## 라이선스와 보안

기여 코드는 [Apache License 2.0](LICENSE)을 따릅니다. 자신이 기여할 권리가 있는 자료만 제출하고, 제3자 코드·문서·자산의 라이선스와 고지를 보존해주세요. [NOTICE](NOTICE)는 RX의 고지이며 외부 의존성의 고지를 대체하지 않습니다. 인증정보·장비 주소·개인정보는 공개 PR이나 이슈에 넣지 않습니다. 취약점 제보는 [보안 정책](SECURITY.md)을 따릅니다.
