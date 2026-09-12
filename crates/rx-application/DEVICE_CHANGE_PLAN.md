# 장비 후보의 공정 변경 계획과 Host별 요구

phase71. 장비 후보 공정의 독립 소프트웨어 승인에서 변경 제안·영향 검토·스테이징까지 연결한다. 실제 Host/native 설정을 바꾸거나 기존 metadata 적용 API를 장비 설치 확인으로 사용하지 않는다.

## 변경 제안

기존 process-change Create에서 승인된 v2 공정 검토를 선택할 수 있다. worker는 공정과 모든 장비 원본을 다시 검증한다. 후보 구성을 계산한 뒤 실제 compiled node ID별 step을 만든다. Intent뿐 아니라 조건·완료 대응표·후조건·condition revision·인계 정책을 후보에서 가져온다. before는 기존 활성 구성이다.

새 Host/resource를 포함한 후보 구성으로 공유 영향의 transitive closure를 다시 계산한다. 생성·commit·현재성 검사와 독립 영향 검토·staging에서 같은 영향을 대조하고 전체 영향 셀 접근권을 요구한다. 후보 구성 계산만으로 active 구성이나 Run은 바뀌지 않는다.

장비 후보가 있으면 Change의 `host_binding_plan`에 다음을 보관한다.

- 설치·셀, 장비 검토 문맥과 공정 검토 버전의 digest.
- before/after CellConfiguration 참조, definition·운영 envelope·environment·scope.
- Host별 실제로 사용할 정규화 Intent와 condition ID 집합.
- 선택한 작업에 대응하는 서명 장비 패키지 manifest/signature와 catalog 참조.
- 해당 Host를 공유하는 다른 영향 셀 목록.

공유 schema는 `rx.host-binding-plan.v1`이다. Host64개, Host당 Intent4096개·condition128개·장비 패키지16개와 전체1,000,000-byte 한도를 둔다. Intent digest와 패키지 identity를 정렬하고 중복을 제거한다. 이 artifact는 S가 비교할 입력이며 설치 명령·실행 허가가 아니다.

기존 device context 없는 Change의 digest 계산은 유지한다. 장비 계획은 기존 plan digest와 Host 요구를 별도 domain으로 결합한다. Stage에서 원본을 다시 검증해 계산한 after 구성·step origins·Host 요구가 검토된 값과 같은지 확인한다.

## 현재 차단 위치

GET의 blockers에 `HOST_BINDING_CHANGE_REQUIRED`를 표시한다. 장비 계획이 있는 변경은 BeginPreparation에서 거부하여 epoch/fence를 먼저 변경하지 않는다. Host 구성 전송과 P 적용의 공통 barrier도 거부한다. 따라서 기존 Host의 공정 문맥 APPLIED_UNQUALIFIED receipt를 native 장비 설정 변경의 증거로 사용할 수 없다.

이후에는 Host의 실제 설치 identity와 원장 세대, 전체 관리 셀, 정지·지지 상태, 변경 효과 및 복원 결과를 durable 절차로 조정해야 한다. 이 phase는 그 적용 절차를 구현했다고 주장하지 않는다.

## S의 기동 설정 비교

[Host 설정 검사 명세](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/HOST_BINDING_INSPECTION.md)를 따른다. 실제 제품 `rx-hostd`가 P artifact와 current/proposed pinned configuration을 비교한다. 검사 결과는 소프트웨어 일치 여부일 뿐 승인된 Host receipt가 아니며 P가 적용 근거로 반입하는 endpoint는 아직 없다.

첫 통합에서는 서로 다른 두 Host가 자원을 공유하는 두 셀 fixture를 사용했다. 장비 변경 Host는 한 셀을 관리하며 두 셀 영향 접근권은 유지한다. 현재 JTC/MELSEC 패키지 backend의 ‘하나의 정확한 셀 binding’ 제약을 풀어 통과시키지 않는다. 여러 셀을 한 native Host에서 지원하는 실제 backend 구성은 별도 검증이 필요하다.
