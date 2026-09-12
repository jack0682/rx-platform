# 유휴 Host의 P 재시작 후 증거 통신 시험

`test_host_reconnect.py`는 새 FILE_SIMULATION 설치에서 P만 종료·재시작하고 H는 같은 프로세스로 유지한다. 새 장비 작업이나 evidence를 만들지 않은 상태에서도 H가 새 P 인증 세션과 Cell.Open 협상을 완료하는지 확인한다. 운전 등록 rebind·자격 복원·재개를 수행하는 도구는 아니다.

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_reconnect.py \
  --platform-image rx-platform:runtime-draft \
  --solutions-image rx-solutions:runtime-draft \
  --evidence-dir ../references/implementation/idle-reconnect-new
```

새 증거 경로가 필요하다. 두 실제 제품 이미지와 기존 자료 exporter/서명 패키지 도구를 사용한다. 초기화는 제품 CLI가 수행하며 원장·Session·HostRegistration 행을 직접 쓰지 않는다. 등록 단말은 생성한 CA와 client certificate를 검증하여 실제 HTTPS 로그인한다. 인증서 검사를 끄지 않으며 시험 인증서에도 authority key identifier를 넣는다. private key·서명 seed·로그인 cookie는 임시 공간에만 둔다.

판정은 다음과 같다.

- P 재시작 전후 installation·store generation·shared clock은 같고 runtime boot는 다르다.
- H instance·boot·evidence/delivery journal·설치 descriptor는 같다.
- 새 producer session은 새 P runtime에 속하며 cell 협상을 마친다. 이후 P evidence cursor revision이 두 번 이상 실제 증가하고 through=0을 유지하는 것과 같은 session의 재사용을 함께 확인한다. 단순 대기만으로 반복 probe를 증명하지 않는다.
- 이전 operating HostRegistration의 원문/버전은 그대로 남고 옛 session에 묶여 있다. 이 통신 복구를 운전 재등록 완료로 표현하지 않는다.
- P의 기존 제한은 보존되고 정확한 RuntimeRestart가 추가된다. 이 시나리오에서 P 재시작만을 이유로 DeviceRestart를 추가하지 않는다.
- 미자격 셀에 자격이나 Run이 생기지 않으며, 독립 FileDevice 효과와 Host evidence 원장의 record 수는 0이다.

내부 producer/session의 대조에는 별도 검증 프로세스가 SQLite를 **읽기 전용**으로 연다. 네트워크를 끄고 데이터 volume을 read-only로 연결한다. 실행 중에는 live WAL의 읽기 transaction을 사용한다. 종료된 P의 경우 실제 container exit를 먼저 확인하고 잔여 WAL이 없거나 비어 있을 때만 immutable 읽기를 사용한다. 읽기 편의를 위해 제품 DB를 수정하거나 운영 API를 우회해 권한을 만드는 경로는 없다.

Host status의 `pending_evidence: null`은 0으로 해석하지 않는다. 해당 값은 종료 단계에서 채워지므로 유휴 시험은 실제 Host evidence 원장 수를 별도 조회한다. API·status·읽기 전용 oracle의 공개 결과를 보존하며, 실패한 실행은 `result.json`의 PASS를 만들지 않는다. 실패 cleanup은 이 도구가 소유한 모의 컨테이너·volume·network에만 적용된다.

이 시험은 미자격 파일 장비와 정상 P 종료/같은 저장소 재시작에 한정한다. 활성 생산 작업·UNKNOWN의 해결, 실제 센서 세대 연속성, H 재시작, 저장소 복원/교체, 권한의 명시적 rebind 및 실물 보호·복구는 별도 검증 대상이다. P의 종료 코드2는 잔여 attention과 함께 기록하고 깨끗한 전체 현장 종료로 확대하지 않는다.

현재 P의 빈 Publish도 cursor revision과 감사 사건을 기록한다. 1초 간격은 유휴 Host당 하루 약86,400회의 감사 기록을 만들 수 있다. 이번 시험은 재연결 의미를 검증하며, 장기 운영의 기록 보존·용량·부하 한계와 probe 전달 방식 최적화는 별도 검증 대상으로 남긴다. Native evidence through가 증가하지 않는 것과 저장 비용이 없는 것은 다르다.
