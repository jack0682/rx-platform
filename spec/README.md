# 규범 원본 보존

두 v1.0 폴더는 RX 설계 작업 공간에서 복사한 기준판이다. 본문과 protocol manifest는 수정하지 않는다. 원문 상대 링크에는 원래 설계 작업 공간의 구조가 남아 있다.

구현 결정과 진행 기록은 workspace의 docs/implementation을 따른다. 이 보존본의 hash 일치만으로 구현 적합성이 증명되지는 않는다.

검사: python3 tools/check_contract_baselines.py

선택 Host 공정 구성 문맥 확장은 [host-configuration/v1](host-configuration/v1/README.md)에 둔다. 기존 base/cell 규범을 수정하지 않으며, native 설정 적용이나 운전 자격을 뜻하지 않는다.

선택 Host 자격 수용은 [host-qualification/v1](host-qualification/v1/README.md)에 둔다. Host 수용과 P 전역 활성화·사용자 시작을 구별한다.
