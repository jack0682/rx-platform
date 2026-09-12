# 검증된 패키지의 보관과 재검증

2026-09-12. 구현: `rx-package::store`, `rx-package::policy`, P 이미지의 `rx-package-store`.

이 단계는 서명된 패키지를 RX가 소유하는 저장소로 가져오는 기반이다. 결과는 `CONTENT_VERIFIED_NOT_ADMITTED`다. 원본 디렉터리를 계속 실행 입력으로 참조하지 않고, 검증 당시 소유한 bytes를 별도로 보관한다. 보관 성공만으로 셀 구성, Run, qualification, Host grant 또는 작업 permit을 만들지 않는다.

## 식별자와 책임

`ObjectId`는 두 SHA-256으로 구성한다.

| 필드 | 계산 대상 | 목적 |
|---|---|---|
| manifest | set 순서를 정규화한 manifest bytes | 패키지 내용과 선언의 identity. manifest가 모든 내용 파일의 digest/size를 포함한다. |
| signature | 정규화한 signature envelope bytes | 서명자 key ID와 detached signature의 identity. 같은 내용의 재서명을 구별한다. |

패키지의 기존 `manifest_digest` 의미를 변경하지 않는다. 같은 내용에 다른 key로 서명하면 manifest identity는 같고 저장 object는 다르다. root 아래 디렉터리 이름은 `<manifest>-<signature>`다. 이름 자체를 신뢰하지 않고 읽을 때 두 digest를 다시 대조한다.

`Store::put`은 공개 검증기를 통과한 `VerifiedPackage`만 받는다. 파일 경로나 Boolean PASS를 받지 않는다. `Store::verify`는 호출자가 신뢰된 구성 경계에서 공급한 **현재 VerificationPolicy**로 저장된 bytes를 새로 취득·검증한다. 과거 성공 결과나 보관 여부를 현재 publisher/permission/dependency/asset의 유효성으로 사용하지 않는다.

P/S 공통 라이브러리는 패키지 외형, 서명, 파일 inventory, 계약·target, 의존성 및 asset 참조를 검사한다. 내용의 공정 의미 검사와 재컴파일은 S의 `compile_verified` 책임이다. 공정 컴파일 성공도 현장 바인딩·실물 검증과 운전 허가를 대체하지 않는다.

## 저장 절차

1. 관리자 소유의 절대 root와 부모 경로를 지정한다. root는 실제 디렉터리여야 하며, 마지막 경로의 symlink를 거부한다. 부모 경로의 관리·mount 구성은 신뢰 전제다.
2. root의 디렉터리 handle을 보유하고 `store.lock`의 독점 잠금을 획득한다. 그 뒤 경로 이름이 바뀌어도 다른 디렉터리로 쓰기가 전환되지 않는다. 이 잠금은 협력 프로세스 간 소유권이며, 같은 OS 계정의 악의적 lock 삭제나 디스크 관리자의 변조를 막는 보안 경계가 아니다.
3. 새 보관소는 `store.json` 종류 표식을 갖는다. 다른 파일이 있는 미초기화 디렉터리를 보관소로 채택하지 않는다. 표식은 임시 파일에 기록·sync한 뒤 no-replace rename으로 공개한다. `open_existing`은 새 root나 lock을 만들지 않는다.
4. `.incoming-<uuid>` 아래에 manifest·signature·내용 파일을 모두 기록한다. regular file만 만들며 실행 권한을 부여하지 않는다. manifest의 executable 표기는 보존되지만 실제 설치 권한으로 적용하지 않는다.
5. 각 파일과 하위 디렉터리, staging 디렉터리를 sync한다. 같은 filesystem 안에서 no-replace rename으로 최종 이름을 공개한 뒤 root를 sync한다.
6. 같은 object를 다시 넣으면 저장된 모든 bytes를 재취득해 정확히 비교한다. 같으면 root sync 후 같은 ID를 돌려준다. 다르면 오류다. 기존 내용을 덮어쓰거나 자동 수리하지 않는다.

임시 파일/디렉터리의 부분 기록은 완료 object가 아니다. 중단 후 남은 `.incoming-*`는 자동 채택하거나 삭제하지 않는다. 표식 초기화 도중의 `store.pending`도 자동 복구하지 않는다. 명시적 점검·정리 정책은 후속 유지보수 기능이다. 최종 공개 후 응답을 잃었다면 같은 서명 패키지를 다시 가져와 내용 비교와 root sync를 거쳐 같은 ID를 회수할 수 있다.

이 절차는 OS의 파일 동기화·rename 보장을 사용한다. 현재 시험은 프로세스 재실행, 중단 잔여물, 손상, 소유권, 경로 변경의 기능 시험이다. 전원 차단, 파일시스템/SSD 장애, 거짓 flush 보고의 내구성을 검증한 결과가 아니다. 첫 지원 구현은 Linux/macOS다. Windows publication backend는 미구현이다.

## 현재 정책의 취득

S에 있던 로컬 policy loader를 공통 `rx-package::policy`로 옮겼다. S의 `trust`는 이를 재수출한다. schema는 `rx.package-verification-policy.v1`를 유지한다.

- key ID/publisher/허용 종류·permission, base/cell hash·package ABI, target을 읽는다.
- key ID 중복을 거부한다. 현재 policy에서 사라진 key를 저장소에서 복원하지 않는다.
- dependency 디렉터리의 파일을 새로 취득하고 정확한 manifest digest와 서명을 확인한 뒤 순서대로 closure를 검증한다. cycle·미해결·중복 이름을 거부한다.
- asset 파일의 실제 bytes를 다시 읽어 SHA-256/size를 검사한다. asset bytes를 이 저장소에 복사하는 기능은 없다. 이후 사용할 때도 별도 immutable acquisition이 필요하다.
- 파일은 no-follow/nonblocking으로 연 뒤 regular file 여부와 크기를 확인한다. JSON 입력은 1 MiB, key128개, dependency128개, asset1024개/총256 MiB다. 각 dependency/패키지는 현재 loader 기준 content4 MiB·32 files이며 metadata 취득 여유2 MiB가 별도다.
- source directory traversal은 깊이24와 내용 파일 수+metadata2개 및 전체 byte 한도를 적용한다. 디렉터리 수는 파일당 최대25 entries로 제한한다. 깊은 정상 경로가 작은 file inventory 때문에 보관 후 읽히지 않는 문제를 피한다.

검증은 한 호출에서 취득한 policy snapshot을 기준으로 한다. 실행 중 trust revision의 실시간 폐기나 P authority의 admission 원자성은 아직 연결하지 않았다. 결과에 표시하는 policy digest는 **policy 파일의 정확한 bytes**에 대한 SHA-256이며, 원장에 등록된 신뢰 revision이라는 뜻이 아니다.

## 플랫폼의 오프라인 도구

P 이미지에 `/usr/local/bin/rx-package-store`를 포함한다. 기본 entrypoint는 기존 `rx-platformd`다. 도구를 명시적으로 실행해야 하며 HTTP API가 아니다. ROS·S compiler·네트워크·장치 권한을 요구하지 않는다. root filesystem은 read-only로, 반입/설정은 read-only mount로, 전용 store volume만 writable로 둔다. root와 부모 mount는 도구 계정 전용으로 관리한다.

```json
{
  "schema": "rx.package-store-config.v1",
  "store_root": "/var/lib/rx/packages",
  "import_root": "/var/lib/rx-import",
  "policy_file": "/etc/rx/package-policy.json",
  "policy_digest": "<policy 파일 bytes의 실제 SHA-256>"
}
```

위 digest 문자열은 자리표시자이며 유효한 설정이 아니다. 실제 policy를 관리자가 공급하고 정확한 hash를 넣어야 한다. 공백 변경도 pin을 바꾼다. CLI는 설정·store/import/policy 및 dependency/asset 경로에 절대 경로를 요구한다. 반입 대상만 `PackagePath` 형식의 상대 경로이며, 각 구성요소의 symlink를 거부한다. 패키지가 자기 trust policy나 임의 출력 경로를 지정하지 못한다.

```text
rx-package-store import /etc/rx/package-store.json tending/package-v1
rx-package-store verify /etc/rx/package-store.json <manifest SHA-256> <signature SHA-256>
```

import는 policy pin과 모든 입력을 검증한 뒤 보관소를 열고 bytes를 저장하며, 저장소에서 다시 읽어 검증한 결과를 출력한다. 검증 실패는 신규 보관소를 만들지 않는다. verify는 기존 보관소를 열어 동일 검증을 수행한다. 성공 stdout에는 object/package/version/kind/policy digest/contracts/target과 `content_semantics_verified=false`, `activation_authorized=false`가 들어간다. 실패 시 nonzero exit다. 오류 또는 stdout 유실만으로 import가 전혀 기록되지 않았다고 단정하지 않는다.

오프라인 도구 실행 권한은 OS 계정 경계다. 현장 계정의 Engineer/Verifier 역할 검사나 사용자 감사 기록을 대신하지 않는다. 운영 writer와 동시에 같은 store를 소유할 수 없다. 현재 도구는 production trust 설치·승인 기능도 제공하지 않는다.

## P 업무 원장 연결과 후속 순서

| 단계 | 추가해야 할 입력·검사 | 저장/원자성 경계 |
|---|---|---|
| 반입 접수 | 현재 사용자/단말·허용 셀, request key/body, object/서명/현재 trust revision | off-writer에서 bytes 검증·보관 후, writer가 현재 권한·trust revision·구성 revision을 다시 검사하여 접수 기록과 응답을 한 transaction으로 저장 |
| 검토 자료 | 불변 source/binding·compiler/version·resolved digest, dependency·asset·장비/현장 context, 검사 항목별 근거 | 독립 worker 결과를 대상 object 및 구체 검토 revision에 결합. 부정/미수행/만료를 구별 |
| 검토 승인 | 현재 검토자 역할, 승인할 정확한 내용·현장 context, 충돌/권한 변화 | server에서 현재성 재검사. 승인 이후 내용 변경은 새 검토가 필요 |
| 활성화 준비 | 운전 중 사용 참조·영향 closure·정리/변경 요건·복원 자료 | 현재 Run의 immutable 참조를 덮어쓰지 않는 변경 절차. 단순 pointer 교체로 구현하지 않음 |
| 활성화·기동 | 현재 qualification·모드·차단·관측·Host 세대·기존 계약의 허가 | 이미 확정된 base/cell 계약의 시작·변경·허가 규칙 적용 |

filesystem publication과 SQLite commit을 하나의 transaction이라고 부르지 않는다. 먼저 object를 완성하고, writer가 그 ID를 참조한 접수 기록을 commit한다. 그 사이 중단되면 **참조되지 않은 object**만 남을 수 있어야 한다. 반대 순서는 durable 접수만 있고 실제 bytes가 없는 상태를 만들므로 금지한다. 원장 rollback이나 백업 복원은 현재 물리 상태와 기존 UNKNOWN을 지우는 근거가 아니다.

반입 접수 행은 [사용자·셀별 반입 API](../rx-application/PACKAGE_INTAKE.md)로 연결했다. 등록된 Store owner와 실제 검증 정책 fingerprint를 사용하고, worker 앞뒤로 현재 권한·문맥을 확인한다. 공정의 검토 자료·소프트웨어 승인은 [PROCESS_REVIEW](../rx-application/PROCESS_REVIEW.md)로 연결했다. 표의 Device/UI·전체 현장 검토와 활성화는 후속 연결 조건이다. application SQLite schema와 frozen 규범 파일은 바꾸지 않았다.
