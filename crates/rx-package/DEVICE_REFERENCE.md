# 장비 구현 참조 패키지 v2

2026-09-12. 패키지 내용 검증의 확장이다. 확정된 base/cell protocol manifest를 바꾸거나 장비 운전 자격을 생성하지 않는다.

## 별도 entry가 필요한 이유

기존 `DEVICE` entry는 서명 inventory에 실행 가능한 adapter 파일을 요구한다. 이미지에 정적으로 포함된 어댑터를 선택하려면 다른 표현이 필요하다. JSON descriptor를 실행파일로 표시해 검사를 통과시키지 않는다. 새 entry는 **`DEVICE_REFERENCE`**이며 `schema=rx.package.v2`, `package_abi=rx.package-abi.v2`를 사용한다.

| 형식 | entry | adapter 자료 | 검증 |
|---|---|---|---|
| v1 | DEVICE | 실행파일 | 기존 요구 유지 |
| v1 | PROCESS / UI | 선언형 내용 | 기존처럼 실행파일 금지 |
| v2 | DEVICE_REFERENCE | 실행 불가 구현 descriptor | 모든 실행 가능 payload 금지 |

현재 v2는 DEVICE_REFERENCE만 지원한다. v1 schema/ABI로 표시한 DEVICE_REFERENCE는 거부한다. 구형 reader는 새 entry를 알지 못하므로 기존 실행파일 adapter로 해석하지 않고 거부한다. 정책도 해당 ABI를 명시해야 한다. 자동 변환이나 migration은 없다.

## 서명에 포함되는 의미

entry는 `family`, 비어 있지 않은 고유한 `profiles` 목록, `adapter`를 가진다. 각 경로는 서명된 inventory의 서로 다른 파일을 가리킨다. publisher kind와 permission 검사에서는 PackageKind::Device이며 Process의 OperationSubmit 권한을 얻지 않는다.

공통 verifier는 서명·key 범위·schema/ABI/target·정규 identity·inventory/bytes/digest·dependency/asset과 기존 경로 경계를 검사한다. `manifest_bytes`는 두 device 형식 모두 profile 경로를 정렬한다. 기존 signing domain을 유지하며 schema/ABI/entry kind 자체가 서명 내용에 포함되므로 새 서명 없이 종류를 바꿀 수 없다.

verifier는 공유 라이브러리를 로드하거나 descriptor를 실행하지 않는다. 드라이버 선택과 장비 의미 판정도 하지 않는다. 제품에 고정된 resolver가 descriptor schema와 정확한 구현 identity, profile과 현재 환경을 검사하고 알 수 없는 구현을 거부해야 한다. 패키지 서명은 출처·내용에 대한 근거이며 물리 qualification이 아니다.

## 호환과 검증

`device_reference_v2_has_explicit_version_and_no_executable_payload`는 서명된 v2, 잘못된 schema/ABI, 금지된 실행 payload, v1의 실행파일 요구 보존을 검사한다. 기존 시험은 서명·권한·경로·원본 소유와 trust 변경을 다룬다. SDK export로 변경된 model/verifier를 solutions에 전달하며 base/cell 규범 파일은 유지한다.

첫 resolver는 [MELSEC Host startup](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/DEVICE_PACKAGE_STARTUP.md)이다. 혼합 v1/v2 dependency 조정은 이 확장에 포함되지 않는다. MELSEC Template/Site 작성·서명 도구는 [rx-device-package](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-device-package/README.md)에 연결했다. 첫 resolver는 package dependency를 허용하지 않는다. 실행 결과와 source hash는 [phase58 검증 기록](../../../references/implementation/phase58_checks.json)에 둔다.
