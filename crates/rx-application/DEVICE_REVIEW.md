# 장비 패키지 소프트웨어 검증·독립 검토

phase65. 반입한 package object와 공통 device catalog에 대해 검증 요청·서명 보고서·독립 검토 이력을 남긴다. 승인 scope는 `DEVICE_PACKAGE_SOFTWARE`다. 교정·실제 제어권·물리 거동의 qualification이나 셀 구성 적용/운전 활성화를 만들지 않는다.

## 하나의 검토 대상

P가 만든 Request는 review/intake ID, 설치·셀, package manifest/signature digest, 정규화 catalog ArtifactRef, 현재 구성 digest, 반입 정책 fingerprint·원본 파일 digest, 장비 검증 authority digest를 고정한다. Job에는 현재 Store owner/registration generation도 보관한다.

Create는 현재 Engineer/Verifier가 수행하며 반입 당시 구성과 현재 구성이 같아야 한다. 구성이 바뀌었다면 원본을 현재 구성 문맥으로 다시 반입한다. 같은 request key의 결과 회수는 현재 신원·역할·셀 확인 후 원래 Job을 돌려준다. 같은 review ID를 다른 내용으로 다시 저장할 수 없다.

## 검증 도구의 실제 검사

`rx-device-package`는 다음 명령을 제공한다.

```text
rx-device-package validator-identity
rx-device-package review PACKAGE POLICY REQUEST OUT_DIRECTORY
rx-device-package review-signing-request REPORT KEY_ID OUT_FILE
```

현재 공통 catalog가 있는 DEVICE_REFERENCE 패키지에 대해 세 검사를 수행한다.

| 검사 | 실제 수행 |
|---|---|
| CONTENT_SIGNATURE | 공통 verifier로 서명·publisher/권한·계약/target·payload·asset 검사 |
| DEVICE_SOURCE_CONSISTENCY | S의 실제 장비 decoder로 assembly/profile/model/release/operations/outcomes/catalog 재대조 |
| CATALOG_REQUEST_BINDING | 요청의 package/catalog·설치/셀과 실제 선언의 상관 확인 |

서명/요청의 근본 상관이 틀리면 도구는 보고서 생성 자체를 거부한다. 신뢰할 원본을 취득했으나 장비 decoder가 실패하면 실패 check와 제한된 issue를 포함한 보고서를 만든다. 세 검사 모두 PASSED이고 issue가 없어야 소프트웨어 승인 후보가 된다.

보고서는 `rx.device-verification-report.v1`이며 scope enum은 DEVICE_PACKAGE_SOFTWARE만 표현한다. 물리 qualification scope나 누락된 check를 성공으로 역직렬화하지 않는다. Report는 최대128KiB/32 issue이며 원래 요청 전체를 포함한다. 도구는 개인키를 읽거나 서명 요청을 외부로 보내지 않는다.

반입 정책과 검증 도구의 추가 취득 한도는 구별한다. 요청에는 P가 실제 사용한 정책 fingerprint를 유지한다. S는 동일 정책의 키/권한/target/asset을 사용하면서 최대8 payload·2MiB라는 더 작은 취득 한도를 적용한다. 더 작은 한도를 적용했다고 반입 정책의 식별자를 바꾸거나 요청의 fingerprint 대조를 생략하지 않는다. validator_policy_file_digest는 S가 읽은 원래 정책 파일의 hash다.

보고서 서명 메시지는 `RX-DEVICE-VERIFICATION-REPORT-v1` 도메인, key ID의 canonical bytes, 보고서 canonical bytes를 결합한다. process 보고서나 package manifest의 서명을 재사용하지 않는다. 외부 signer는 signing request의 hex를 실제 bytes로 복원해 Ed25519 서명하고, 기존 SignatureEnvelope 형식의 verification.sig.json을 제공한다.

## P 저장과 승인

장비 검증 authority는 process authority와 별도 설정 파일이다. schema는 `rx.device-verification-authority.v1`이며 key ID/public key별 허용 validator digest를 명시한다. 파일 pin·의미 digest를 시작 때 확인하고 report/approval worker가 다시 읽는다. 패키지가 자기 검증 키를 설치할 수 없다.

P worker는 등록된 Store에서 원본 package를 현재 정책으로 재검증하고, 보고서 요청·catalog bytes·검증 signer/validator를 확인한다. 파일/암호 작업은 기존 단일 semaphore의 bounded worker에서 수행한다. 그 결과는 역직렬화할 수 없는 Prepared token으로 writer에 전달한다.

writer는 현재 역할·셀·구성/registration/authority·boot/30초 ticket과 expected revision을 다시 검사한다. 보고서/서명/버전 digest·history·사건·동일 key 결과를 한 transaction에 기록한다. 새 보고서 버전은 이전 승인과 자동으로 연결되지 않는다. 과거 버전은 읽을 수 있다.

APPROVE는 Verifier 역할이며 package 제출자와 다른 계정이어야 한다. 정확한 최신 report revision/review digest, 현재 checker digest와 모든 소프트웨어 check 통과를 요구한다. 승인 직전에 worker가 package·정책·authority·서명을 다시 검증하며, writer commit 직전에도 같은 문맥인지 확인한다. 도중 정책/보고서/역할이 바뀌면 승인하지 않는다. REJECT는 최신 대상을 명시하면 현재 authority가 철회된 상황에서도 기록할 수 있다.

결정과 history·event·request-key 결과는 원자 기록한다. 응답 유실 후 같은 요청은 원래 결정을 회수한다. 그 뒤 정책이 철회되었을 때 과거 결정을 회수했다고 현재 승인을 복원하지 않는다. 조회의 approval_matches_current_review는 현재 등록된 문맥·최신 보고서와의 일치이고, 실제 파일을 방금 재검증했다는 뜻은 아니다. 후속 구성 적용은 별도의 현재 원본 검증을 요구해야 한다.

## API·배포 경계

| 경로 | 기능 |
|---|---|
| POST /api/v1/device-reviews | Job과 검증 요청 생성 |
| GET /api/v1/device-reviews?cell=…&intake=… | 50개씩 조회·다음 cursor |
| GET /api/v1/device-review?cell=…&id=…&revision=… | 최신 또는 과거 보고서/결정/현재성 |
| POST /api/v1/device-review/reports | 파일 경로·report digest·expected revision으로 반입 |
| POST /api/v1/device-review/decisions | 정확한 버전 승인/반려 |

모든 mutation은 기존 request_key wrapper와 현재 인증을 사용한다. HTTP와 단말 HTTPS는 같은 application을 호출한다. 새 gRPC 서비스나 frozen wire 계약은 추가하지 않았다. package-intake-context에는 별도의 device_review_authority_digest를 제공한다.

rx-platformd의 package_intake.device_review_authority에 pinned 파일을 지정한다. 없으면 장비 검토 Job을 생성할 수 없다. 개발 전용 local service에도 같은 선택 필드를 제공하지만 production trust로 기본 활성화하지 않는다. 검토 상태를 조작하는 브라우저 UI는 후속이며 이 단계의 통합 시험은 실제 API를 호출한다.

실제 검증 결과는 [phase65 증거](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase65_checks.json)를 따른다. 이 단계는 device validator의 코드/서명 공급망을 하드웨어 attestation한 것이 아니며, 신뢰 등록한 signer가 지정된 도구 결과에 서명한다는 경계다. JTC production Authority/lifecycle/fencing, 장비 검토 UI, 승인된 작업의 셀 구성 변경·qualification·물리 인수는 남아 있다. 첫 물리 셀은 NOT_COMMISSIONED다.

phase66에서 [장비 검토 화면](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/apps/operator/DEVICE_REVIEW_UI.md)을 연결했다. 목록은 report 전체가 아닌 Summary50개를 반환하며, 상세/과거 버전 API는 유지한다. 기존 phase65 목록 소비자는 요약 형식으로 갱신해야 한다. UI는 현재 버전에 결합한 확인창과 기존 pending 요청 회수를 사용한다. 실제 구성 변경·물리 qualification은 계속 후속이다.

phase67에서 현재 승인/원본을 다시 검증하는 [작업 연결 변경안](DEVICE_BINDING_PLAN.md)을 추가했다. 조건/Host·후보 자원 영향과 독립 검토를 기록하며 실제 configuration/qualification/Run은 변경하지 않는다. 후보를 공정 재검증·Host binding 변경에 결합하는 적용 절차는 후속이다.
