# 장비 작업 선언 반입·보관·조회

phase64. 서명된 DEVICE_REFERENCE 패키지 안의 공통 작업 선언을 기존 package intake에 연결한다. 선언 내용을 사람이 다시 입력하지 않고, 원본 package object와 연결하여 보관·조회한다. 이 단계는 제조사별 장비 검증 보고서 승인이나 셀 구성 적용이 아니다.

## 공통 선언과 제조사 책임

공유 `rx-process-contract::device_catalog::Catalog`에는 설치·셀·환경·target/profile digest, 필요한 condition ID, 작업별 Intent, optional native 결과표와 source 문서 참조가 있다. 최대64작업·32조건, 선언 전체128KiB를 허용한다. 필수 문서 역할은 family/profile/adapter/operations이며 결과표가 있으면 outcomes도 요구한다. 각 문서는 package 안 경로와 ArtifactRef로 연결한다.

S의 JTC 작성기는 기존 assembly에서 profile/operations/outcomes를 만들면서 `device-catalog.json`도 생성한다. Host의 JTC decoder는 이 선언을 동일 assembly에서 재계산해 대조한다. P는 ROS나 제조사 profile 구조를 import하지 않으며 공통 선언의 상관과 원본 bytes만 검사한다.

P는 verified package에 선언 파일이 있을 때 다음을 검사한다.

- DEVICE_REFERENCE entry의 family/adapter/profile 경로와 선언이 일치한다.
- 각 문서 경로는 유효한 PackagePath이며 실제 signed payload의 hash/size와 일치하고 실행파일이 아니다.
- 작업 목록과 결과표가 참조한 원본 문서와 같고 모든 Intent의 target/profile/rule이 선언과 맞는다.
- 현재 설치 ID·반입 셀·SIMULATION/PHYSICAL 환경이 맞는다.

제조사 profile의 의미·교정 적합성·제어권·실물 거동은 이 공통 검사로 검증되지 않는다. API는 항상 `manufacturer_validation_required=true`, `activation_authorized=false`를 반환한다. 선언을 실제 StepBinding으로 설치하거나 qualification/Arm/Run을 생성하는 endpoint는 추가하지 않았다.

## 영속성과 현재성

파일 취득·서명·asset 검증은 기존 bounded package worker가 writer 밖에서 수행한다. `Prepared`는 실제 등록된 Store의 immutable object와 ticket을 결합하며 wire에서 역직렬화할 수 없다. 서명 검사를 통과해도 선언이 원본과 모순되면 Prepared를 만들지 않는다.

authoritative commit은 현재 신원·역할·셀·설치·policy registration·구성 digest·ticket 만료를 다시 검사한다. 정규화한 선언, 그 ArtifactRef, receipt·원본 object ID·event·request-key 결과를 같은 transaction에서 저장한다. commit 전 실패는 receipt/선언을 남기지 않고, 응답 유실 후 같은 key는 원래 receipt를 반환한다. 파일 Store에는 이미 취득한 object가 남을 수 있으나 그것이 반입 승인이나 실행 권한을 만들지 않는다.

receipt의 optional `device_catalog`는 정규화된 투영의 참조다. 원래 서명된 payload는 receipt.object가 가리키는 package Store에서 보존한다. 조회는 저장된 선언을 다시 serialize/hash/size 검사하고 receipt·설치·셀의 상관을 확인한다. 새로운 설정으로 과거 선언을 덮어쓰지 않는다.

`review_context_current`는 현재 셀 구성과 package service registration이 접수 당시와 같은지만 뜻한다. 재시작/정책 재등록/구성 변경 뒤에는 false가 될 수 있다. 조회 시 원본 Store를 다시 검사하는 것은 아니므로 `content_reverification_required=true`를 유지한다. 과거 내용을 볼 때도 현재 로그인·셀 접근권과 Engineer/Verifier 역할을 요구한다.

## API와 화면

```text
GET /api/v1/package-intake/device-catalog?cell=CELL&id=INTAKE_ID
```

반환값은 cell/intake/object/reference/catalog와 위의 현재성·검증 필요 flags다. catalog가 없는 과거 패키지는 reference/catalog=null로 반환한다. 기존 receipt는 새 optional 필드가 없어도 읽는다. 공통 wire 규범8개와 optional binding6개는 변경하지 않았다.

운영 앱의 패키지 검토 화면은 DEVICE_REFERENCE 형식을 인식한다. 장비 선택 시 대상·환경·조건·작업 자원/시간·결과 대응표·원본 참조를 읽고 자료를 내려받을 수 있다. 장비 패키지에 공정 검토 요청 버튼을 표시하지 않는다. 응답의 셀/intake/object/reference가 선택 기록과 다르면 자료를 현재 내용으로 수용하지 않는다. 늦은 응답은 기존 generation/abort 제어를 따르고, 만료/오류·정책 불일치는 보관 자료임을 표시한다.

## 검증과 후속

시험은 원본/대응표 불일치·경로/해시 변조·설치/환경/셀·ticket 만료, 원자 commit/응답 유실·권한·과거 receipt를 다룬다. 브라우저 통합은 실제 S JTC CLI와 외부 test signer가 만든 패키지를 실제 P Store/API로 반입하고, 다운로드한 선언이 signed package의 선언과 같은지 확인한다. 셀 구성 digest·운영 상태가 바뀌지 않는 것도 확인한다. 정확한 최종 결과는 [phase64 기록](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase64_checks.json)에 둔다.

다음은 제조사 validator의 검증 근거와 독립 검토를 장비 package object·선언 digest에 묶고, 승인된 작업을 셀 구성 변경 계획으로 연결하는 단계다. 현재 이 화면은 선언 조회이며 장비 승인 화면을 완료한 것으로 세지 않는다. production JTC Authority/lifecycle/fencing과 실물 교정/지지·인수는 계속 미완료이며 첫 물리 셀은 NOT_COMMISSIONED다.

phase65에서 [장비 소프트웨어 보고서·독립 승인 API](DEVICE_REVIEW.md)를 추가했다. 별도 authority와 실제 S decoder 보고서를 현재 원본에 대조한다. 이 선언 조회 응답은 해당 승인 상태를 집계하지 않으며, 승인 UI·실제 구성 적용 및 물리 qualification은 후속이다.
