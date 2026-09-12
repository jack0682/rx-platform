# Application ↔ protocol 변환

`rx-protocol-adapter`는 Protobuf와 application 모델 사이의 변환을 소유한다. `rx-domain`/`rx-application`에는 생성된 wire 타입을 넣지 않는다. P가 받는 `Evidence.Publish`와 P가 요청한 `Host.Reconcile` 응답은 같은 `evidence_batch` 변환을 사용한다.

현재 native-result source의 필수·선택 필드를 모두 보존한다. 장비 고유 native_id와 native_data ArtifactRef도 `NativeEvidence.native_details`에 남긴다. 이 부분까지 immutable evidence 비교에 포함하므로, 같은 journal/seq에서 첨부 자료만 바뀌는 경우도 충돌이다.

`native_details=None`인 과거 내부 projection은 완전한 원본을 보존했다고 주장하지 않는다. 이를 `Evidence.Get`의 완전한 wire 원본으로 재구성하려 하면 거부한다. 누락된 정보를 임의로 채우거나 기존 기록을 덮어쓰지 않는다. 이전 binary의 새 내부 필드 읽기/rollback은 별도 호환 검토 대상이다.

현재 지원하는 외부 증거는 `rx.native-result.v1`이다. 다른 EvidenceBody를 NativeResult로 낮추거나 버리지 않고 거부한다. observation/attestation/cancel evidence의 형식과 권한 연결은 후속 범위다.

두 단위 시험은 native_id/native_data 및 JavaScript 정수 정밀도를 넘는 uint64의 무손실 왕복과 legacy projection의 원본 사칭 거부를 검증한다.
