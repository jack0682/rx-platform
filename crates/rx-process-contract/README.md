# 공통 공정 의미 모델

선언형 ProcessSource/ResolvedProcess와 bounded validation·frontier 규칙을 공유한다. ROS·BT·network/filesystem/package loader에 의존하지 않는다. S compiler와 P application은 이 crate를 함께 사용한다. S에는 SDK source bundle로 제공하며 P 업무 authority engine을 수출하지 않는다.

Node/source identity, 구조 한도, binding과 resource 병렬 충돌을 검증한다. Frontier는 완전하고 일관된 progress만 받는다. UNKNOWN/상충/미해결을 실패로 낮추지 않고, 선택되지 않은 branch 이력이나 빠진 predecessor를 거부한다.

이 crate 자체는 권한을 부여하지 않는다. P가 현재 권한·명령 동일성·조건·checkpoint를 transaction에서 확인해 결과를 기록한다. S의 compiler/XML과 C++ BT는 P의 결정과 admission을 대체하지 않는다.
