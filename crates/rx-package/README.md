# RX 패키지 내용·신뢰 검증

장비·공정·UI 패키지의 공통 외형과 내용 신뢰 경계다. Rust 라이브러리이며 ROS·장비 SDK·P 업무 engine에 의존하지 않는다. S에는 SDK의 일부로 같은 구현을 수출한다.

`VerifiedPackage`는 **서명·내용·타깃·의존성 검증을 통과한 immutable bytes**다. 코드 품질·장비 동작·현장 qualification이나 native 실행 권한을 뜻하지 않는다. 검증 함수는 프로세스나 스크립트를 실행하지 않는다.

## 구성

- `manifest.json`: schema/package/version/publisher, 두 계약 hash·package ABI, target, entry, permissions, dependencies, assets, files.
- `manifest.sig.json`: signer key ID와 Ed25519 detached signature.
- 나머지는 file inventory에 정확한 path/SHA-256/size/executable 의미로 등록한다. 대형 ONNX·지도 등의 자료는 독립적으로 검증한 ArtifactRef dependency로 연결할 수 있다.

signature는 `RX-PACKAGE-MANIFEST-v1` domain, key ID, 정규화한 manifest bytes에 결합한다. key 이름만 바꾸어 같은 공개키의 다른 권한 등록을 이용하지 못한다. package digest는 정규화한 manifest의 SHA-256이며, 서명 교체와 내용 identity를 구별한다.

JCS를 쓰며 files/targets/permissions/dependencies/assets/profile 목록은 의미상 set 순서를 정규화한다. 중복·case alias·self dependency·모호한 entry 역할은 거부한다. 버전은 SemVer이고 build metadata로 같은 버전을 다르게 포장하지 않는다.

target의 ros_distribution=None은 ROS 런타임을 요구하지 않는다는 뜻이다. ROS가 설치된 이미지에서도 사용할 수 있다. ROS를 요구하는 패키지는 배포판을 명시하고 일치 여부를 검증한다. 비의존 패키지를 ROS 버전에 불필요하게 결합하지 않는다.

Ed25519 검증은 [ed25519-dalek](https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/)의 `verify_strict`를 사용한다. 이 라이브러리 사용과 자체 시험은 외부 보안 감사나 납품 보안 인수를 대신하지 않는다.

## 권한과 의존성

| 요청 | 허용 package 종류 |
|---|---|
| ArtifactRead | Device/Process/UI |
| ObservationRead(schema) | Device/Process |
| OperationSubmit(operation) | Process |
| NativeEndpoint(role) | Device |
| UiPanelRead(topic) | UI |

요청은 종류별 규칙과 signer의 허용 집합 모두에 속해야 한다. 이 검사는 실행 권한을 실제 부여하는 OS sandbox/broker가 아니다. endpoint role의 실제 장치/주소 binding, site 승인·Host grant/permit은 별도다. UI/공정의 요청을 native 접근 권한으로 승격하지 않는다.

의존성은 이름·정확한 버전·종류·manifest digest로 고정한다. 전이 closure에서도 같은 이름의 다른 버전·root 재유입·cycle을 거부한다. 기존 VerifiedPackage도 현재 target/contract와 signer trust를 다시 확인한다. 이전에 검증됐다는 이유로 회수된 key를 신뢰하지 않는다. 현재 graph 한도는 깊이32/총1,024개다.

asset catalog는 신뢰된 composition 경계에서 제공해야 한다. 임의 API 사용자에게 policy, trust key 또는 asset 검증 결과를 받아서는 안 된다.

## 파일 취득

`directory::verify_directory`는 [cap-std](https://docs.rs/cap-std/latest/cap_std/fs/struct.Dir.html)의 지정 디렉토리 capability에서 읽는다. 부모 이동·절대 경로·Windows reserved name·역슬래시·case/hierarchy 충돌을 금지한다. symlink를 따르지 않고 regular file만 받는다. Unix 파일 열기는 nonblocking으로 설정하여 FIFO 교체에 의한 무기한 open을 피한다.

파일/디렉토리 수·깊이·전체 byte를 제한하고, 검증 후에는 원본 path가 아닌 소유한 bytes를 사용한다. 원본 파일이 나중에 바뀌어도 VerifiedPackage는 바뀌지 않는다. 메모리에 보관하는 패키지이므로 대형 asset streaming/store는 별도 구현 대상이다.

## 보관·로컬 검증 정책

검증된 bytes의 독점 소유 보관소와 현재 정책 재검증을 [STORE.md](STORE.md)에 정리했다. P 이미지의 오프라인 가져오기 도구와 S의 서명 도구가 같은 policy loader를 사용한다. 저장 결과는 CONTENT_VERIFIED_NOT_ADMITTED이며 승인·활성화가 아니다.

단일 ABI 정책은 `rx.package-verification-policy.v1`을 유지한다. 장비 참조 패키지 ABI v2와 공정 패키지 ABI v1을 같은 보관소에 반입할 때는 `rx.package-verification-policy.v2`와 명시적인 `additional_package_abis`를 사용한다. 기본 ABI는 `contracts.package_abi`이며 추가 목록은 1–8개, 중복이나 기본 ABI의 재기재를 허용하지 않는다. v1 문서는 추가 ABI를 허용하지 않고 v2 문서는 빈 목록을 허용하지 않는다.

예를 들어 기본 ABI가 `rx.package-abi.v2`인 정책에 `additional_package_abis: ["rx.package-abi.v1"]`을 지정할 수 있다. 이 목록은 ABI 호환성의 허용 범위만 넓힌다. 두 계약 hash, manifest 종류·schema, target, signer의 종류·권한, 내용·서명 검사는 그대로 적용된다. 추가 ABI가 없는 정책의 fingerprint는 기존 값과 같으며, 추가 목록의 변경은 fingerprint를 바꾼다. 서로 다른 패키지를 받을 때마다 운영 정책을 교체할 필요는 없지만, 정책 자체의 변경은 기존 현재성 검사를 따른다.

## 미완료 경계

- 실제 DeviceFamily/Profile·ProcessSource·UI descriptor의 semantic validation과 실행 연결.
- production trust key 공급/회수·signing service와 독립 검증 보고서.
- 사용자/셀별 stage 접수는 [반입 API](../rx-application/PACKAGE_INTAKE.md)에 연결했다. 공정의 서명된 검토 자료·소프트웨어 승인도 [검토 API](../rx-application/PROCESS_REVIEW.md)에 연결했다. Device/UI·절차 검증과 install/activate, site permission 승인, OS process sandbox/FD broker는 미완료다.
- 이미지/패키지 혼합 배포·변경 영향·qualification과 업데이트/복원.

현재 필드/검증은 새 패키지 외형의 초안이며 frozen 작업·셀 계약 파일을 변경하지 않는다. entry 파일이 존재한다고 그 내용을 실행 가능한 공정이나 검증된 장비로 취급하지 않는다.

공정 패키지의 결정적 조립과 외부 detached signature 도구는 [S process-package](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-process-package/README.md)에 연결했다. 일반 production trust 공급/활성화는 여전히 별도다.
