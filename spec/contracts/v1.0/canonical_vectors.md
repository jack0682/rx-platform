# 정규 표현의 문서 시험 벡터

RX v1.0의 후속 Rust/C++ 구현 시험 입력이다. 아래 SHA-256은 제시된 정확한 UTF-8 바이트로 계산했다. JCS 구현이나 protobuf 상호운용을 실행 검증한 결과가 아니다. 숫자 조각은 전체 Intent가 아니므로 admission할 수 없다. CV01의 profile/시간값도 설명용이며 실제 장비 설정이 아니다.

## CV01 전체 Intent

정규 payload (끝에 newline 없음):

```json
{"body":{"predicate":{"predicate_id":"fixture.closed","settle_ms":"0","target":{"boolean":true}}},"calibration_digests":[],"cancel_rule":"example/stop","completion_rule":"example/closed","execution_timeout_ms":"5000","kind":"ENSURE_STATE","prepare_validity_ms":"1000","profile_digest":"0000000000000000000000000000000000000000000000000000000000000000","resource_set":["controller/example"],"site_config_digest":"1111111111111111111111111111111111111111111111111111111111111111","target":"example/fixture"}
```

앞에 ASCII `RX-INTENT-v1`과 LF 1byte를 붙인 SHA-256:

```text
e94f6df366963718e61e68018e10b1fb0d453e36143119c1e1b59f07d53ddf92
```

## CV02 숫자 정규화 조각

정규 payload (끝에 newline 없음):

```json
{"real":0}
```

앞에 ASCII `RX-INTENT-v1`과 LF 1byte를 붙인 SHA-256:

```text
d5dc68a13b5d1ebd2b2ecea05fe2b14022fea142152458f74e0311d21b0306c8
```

## CV03 명시 false 조각

정규 payload (끝에 newline 없음):

```json
{"boolean":false}
```

앞에 ASCII `RX-INTENT-v1`과 LF 1byte를 붙인 SHA-256:

```text
f378924571af64a661c8a74c7f675d6c9d64416276deefd8b3cd74297a806e2e
```

## 반드시 별도 검증할 입력 쌍

- CV01의 object key 순서 변경은 같은 바이트로 정규화한다. resource 집합 중복은 거부한다.
- CV02의 입력 real=-0.0 및 real=0.0은 같은 바이트다. NaN/Infinity는 거부한다.
- CV03은 false를 명시한 값이다. 필수 value arm 누락이나 null과 같지 않다.
- calibration/tool/profile 변경은 다른 의도다. 같은 key이면 KEY_CONFLICT다.
- Evidence/Receipt는 본문에 정한 별도 domain prefix를 사용하므로 같은 JSON이어도 Intent digest와 같을 것을 기대하지 않는다.
- unknown protobuf field, duplicate singular/map field, 둘 이상의 oneof arm은 decode 전 거부한다.
- fixture 번호는 문서 사례 ID이며 장비 검증 인증 번호가 아니다.
