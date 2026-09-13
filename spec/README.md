# 규범 원본 보존

두 v1.0 폴더는 [RX 문서 저장소](https://github.com/jack0682/rx_docs)에서 복사한 기준판이다. 2026-09-14 제조사 중립 문서 개정을 반영했다. Wire 번호와 semantic version은 유지하며 문서와 manifest hash는 새 개정의 식별자로 갱신했다. 변경 근거와 호환 영향은 [개정 기록](contracts/v1.0/revision_2026-09-14.md)을 따른다.

구현 결정과 진행 기록은 [현재 구현 문서](https://github.com/jack0682/rx_docs/tree/main/docs/implementation)를 따른다. 이 사본의 hash 일치만으로 구현 적합성이 증명되지는 않는다. 이후 수정은 문서 원본의 명시적 개정과 동기화를 통해 반영한다.

검사: python3 tools/check_contract_baselines.py

선택 Host 공정 구성 문맥 확장은 [host-configuration/v1](host-configuration/v1/README.md)에 둔다. 기존 base/cell 규범을 수정하지 않으며, native 설정 적용이나 운전 자격을 뜻하지 않는다.

선택 Host 자격 수용은 [host-qualification/v1](host-qualification/v1/README.md)에 둔다. Host 수용과 P 전역 활성화·사용자 시작을 구별한다.
