# 장비 변경 후보를 포함한 공정의 소프트웨어 검토

phase70. 승인된 장비 패키지에서 만든 작업 후보를 공정 패키지의 출처와 대조하고, 독립적인 공정 소프트웨어 검토 결정을 기록한다. 셀 구성·Host 설정·운전 권한을 변경하지 않는다.

## 고정하는 문맥

Create의 선택적 `device_plans`는 정확한 ID/revision/plan digest 목록이다. 기존 공정 초안과 동일한 catalog 생성기를 사용하여 IMPACT_REVIEWED, issue 없음, 현재 장비 승인·구성·영향 범위와 모든 영향 셀의 접근권을 확인한다. 선택한 plan은 실제 binding selection에 쓰여야 한다.

후보가 있으면 요청은 `rx.process-review-request.v2`이며 `device_context_digest`를 포함한다. Job의 `device_context`는 catalog digest와 각 plan·원래 장비 검토 Job·보고서 버전·독립 결정의 snapshot이다. 최대16개, ID 오름차순, 전체 canonical bytes 512 KiB 이하다. 요청의 digest와 서명된 S 보고서가 이 snapshot digest를 결합한다.

기존 v1은 context 필드 없이 유지한다. v1에 context digest를 넣거나 v2에서 빼면 거부한다. 기본 구성을 후보 구성으로 덮어쓰지 않고, 독립 검사 중에만 후보 step을 합친 별도 구성을 계산한다. 원래 구성을 보존하므로 이후 실제 적용 절차의 before 근거를 잃지 않는다.

## 접수·승인 시 원본 재검증

worker는 공정 패키지와 모든 장비 패키지를 같은 Store owner·현재 policy로 다시 검증한다. 별도로 고정한 장비 authority 파일과 서명·validator·report digest를 확인한다. 과거 DEVICE_PACKAGE_SOFTWARE 승인만 보고 원본 재검증을 생략하지 않는다.

P는 검증한 장비 catalog와 원래 선택 입력으로 candidate를 다시 생성한다. 조건, 완료 결과 대응표, 후조건, Host/Intent, condition revision, 인계 유효기간과 이전 step digest가 보관된 plan 후보와 같아야 한다. 후보 재생성은 최초 plan 작성과 같은 함수를 사용한다.

공정의 signed compile-input 안에서 alias별 plan ref·binding ID·step digest·action digest를 정확히 대조한다. composite catalog도 원래 구성·정렬된 plan refs·합친 step으로 재계산한다. 출처 누락·변경·다른 후보로의 교체를 허용하지 않는다. 기존 source/resolved 구조·권한·조건·predecessor 검사는 후보 구성을 대상으로 계속 수행한다.

writer는 commit 직전에 다시 현재 plan revision/내용, builder, 승인 보고서/결정, authority, 구성·transitive 영향·등록 정책·계정 권한과 ticket의 boot/30초 유효기간을 검사한다. 전체 영향 셀 접근권은 생성·보고서·결정뿐 아니라 과거 조회·동일 key 회수에도 적용한다. 목록은 접근권이 없는 의존 문맥의 Job을 노출하지 않는다.

장비 승인을 새 결정으로 철회하면 기록된 공정 승인과 보고서는 역사적 자료로 남고 `context_current`/`approval_matches_current_review`는 false가 된다. 같은 key의 과거 결정 회수는 그 결정의 재발급이나 현재 승인 복원이 아니다.

## 적용과 화면

이 단계의 승인 scope는 PROCESS_PACKAGE_SOFTWARE다. 현행 process-change는 device context가 있는 Job을 거부한다. 후보의 Host/native 설정 변경과 운영 envelope, quiet/fence·원장 세대 및 적용 후 qualification을 연결한 다음 이 경계를 확장해야 한다. activation_authorized는 false다.

기존 운영 화면 decoder는 v2 요청과 후보 문맥을 보존하며 후보가 빠진 자료를 거부한다. 검토 근거에 후보 자료를 표시하고 현재 셀 설정이 변경되지 않았음을 안내한다. 새 장비 plan을 고르는 전용 작성 화면은 후속이며 이 단계의 생성은 API 경로다.

## 확인한 범위

실제 서명 JTC 장비 패키지·공정 패키지와 S compiler 보고서를 P API에 반입하고 독립 승인을 확인했다. 일부 셀만 접근 가능한 계정, 제출자의 자기 승인, 변경된 장비 authority 파일을 거부한다. 장비 승인 철회 뒤 공정 현재성이 사라지고 새 승인이 차단되며 과거 동일 요청만 회수됨을 확인했다. 전체 소스 경계와 로그는 [phase70 기록](https://github.com/jack0682/rx_docs/blob/codex/initial-draft/references/implementation/phase70_checks.json)을 따른다.

실물 검증·production 서명 서비스·JTC 운전 제공자·현장 적용 인수를 증명한 결과가 아니다. 첫 물리 셀은 NOT_COMMISSIONED다.
