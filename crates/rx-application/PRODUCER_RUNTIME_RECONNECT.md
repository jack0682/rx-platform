# P 재시작으로 인한 evidence producer 세션 교체

`OpenEvidenceProducer`는 현재 P runtime에서 같은 Host boot/evidence journal/authentication binding으로 재연결하면 기존 session을 반환한다. P만 재시작해 session.runtime_boot가 달라졌을 때는 새 evidence session이 필요하다.

이 경우 이전 producer의 principal·Host boot·evidence journal·authentication binding, 이전 active service session의 principal·shared clock·유효성이 같고 runtime_boot만 바뀌었는지 검사한다. 이어 해당 Host의 **모든 기존 등록 셀**에서 현재 P runtime이 이전 session의 runtime_boot를 철회하며 만든 정확한 latched RuntimeRestart block과 immutable origin을 확인한다. origin의 installation/store generation·cell·block·before/after boundary가 유효해야 하며 현재 configuration digest와 전체 scope key coverage도 일치해야 한다.

이 근거가 모두 있으면 새 DeviceRestart를 추가하지 않는다. 이미 있는 RuntimeRestart·AuthorityRevoked·다른 block, cell revision/epoch/scopes·qualification·Run·mandate·permit·work·HostRegistration·grant는 그대로다. 새 producer session을 만들고 이전 session을 비활성화하며 cell 협상 map을 비우는 기존 인증 처리만 수행한다. 새 Cell.Open과 evidence prefix 검증은 여전히 필요하다.

`OpenEvidenceProducer`의 입력에는 source generation이 없다. 이 분기는 source 연속성을 평가하거나 source만 바뀐 사건을 분류하지 않는다. 실제 source 변화의 판정·fact 무효화·DeviceRestart fencing은 기존 observation 경로의 책임이다. 그 경로가 이미 저장한 source DeviceRestart block, 불량 fact, 원래 evidence와 source-generation-loss 기록, 과거 source에 고정된 HostRegistration은 P runtime evidence 재접속 뒤에도 그대로 보존한다. source 변경이나 operating rebind를 이 세션 판정으로 승인하지 않는다.

정상 P stop은 AuthorityRevoked와 lifecycle을 저장하지만 evidence service Session.active는 바꾸지 않는다. 그러므로 정상 stop→동일 DB/clock의 P restart도 이 좁은 판정 대상이며, 정상 stop의 제한도 삭제하지 않는다. 이전 세션이 이미 비활성·만료 상태이거나 Host/auth/journal/clock이 바뀌었거나 origin/현재 boot/configuration/전체 셀·scope coverage가 없으면 기존 DeviceRestart 철회 경로다. 손상되거나 다른 설치/셀에 속한 origin은 무결성 오류로 transaction을 rollback한다.

옛 producer session이 현재 origin의 직전 runtime보다 더 오래되었다면 origin chain을 추정하지 않는다. 반대로 각 P restart 사이에 evidence 재협상을 완료한 경우에는 새 직전 session과 현재 origin을 매번 대조할 수 있다. 이때도 operating HostRegistration의 옛 session은 자동 rebind하지 않는다.

이 변경은 사건 원인의 좁은 구분이다. operating HostRegistration rebind, grant acquire/renew, qualification 복원·활성화, Arm, StartRun 또는 native 제출을 추가하지 않는다. base/cell 규범·SDK·공개 wire도 바꾸지 않는다.

전용 transaction 시험 소스는 정상/비정상 P 종료 뒤 실제 producer session 교체, 등록 셀 전체와 authority record·outbox의 byte-equivalent 보존, Host/journal/auth/clock·origin/범위 반례, commit 이전 실패·commit 응답 유실, 연속 P restart, 기존 source-generation 철회의 fact·evidence·등록·block 보존을 다룬다. 실제 P/H 프로세스의 유휴 publisher 재접속은 별도 통합 시험 범위다.
