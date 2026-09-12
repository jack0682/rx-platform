# 이미 설치된 공정 구성의 재검증 진입점

초기 bootstrap에 compiled process가 있지만 그 구성의 적용 Change 기록이 없으면 `ProcessChange.Propose`의 `mode: "REVALIDATE_CURRENT"`로 검증·승인된 현재 구성에 대한 root를 명시적으로 만든다. 입력을 생략한 기존 mode는 `REPLACE`이며 기존 JSON에서 mode를 생략한다. 기존 REPLACE plan digest 계산은 유지하고 REVALIDATE_CURRENT만 별도 mode digest에 결합한다.

REPLACE는 검증한 target이 현재 구성과 같으면 계속 거부한다. REVALIDATE_CURRENT는 현재 compiled process가 존재하고 검증한 target 전체가 현재 구성과 정확히 같아야 한다. 같은 현재 구성을 포함하는 APPLIED_UNQUALIFIED 또는 QUALIFIED_ACTIVE 적용 root가 있으면 Busy로 거부하며 기존 root를 재사용한다. 이 검사는 제안 시와 commit 시, 이후 stage·준비·적용의 현재성 확인에도 수행한다. 같은 요청 key의 기존 결과 회수는 유지한다.

API Change의 `mode: "REVALIDATE_CURRENT"`와 같은 before/after artifact가 구성 교체가 없었음을 나타낸다. 제안, 독립 영향 검토, stage, 기존 fence 확인, Host metadata ACK를 모두 거쳐야 APPLIED_UNQUALIFIED가 된다. 적용은 새 자격이나 운전 허가를 만들지 않고 블록을 직접 해제하지 않는다. device provenance가 Host binding 변경 준비를 요구하는 기존 차단도 유지한다.

기존 source worker가 같은 Prepared 검증 경로를 사용하므로 별도 우회 worker나 bootstrap 전용 승인 경로는 없다. 전용 시험은 기존 wire/plan 유지, exact compiled target, 독립 검토·fence·Host ACK 요구, 미자격 적용, 동시 제안의 기존 root 재사용, commit 전후 오류와 같은 요청 회수를 다룬다. 실장비 검증과 자격 발급은 이 범위에 포함하지 않는다.
