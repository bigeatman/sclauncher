# 검증 기록 구분

현재 native 배포본은 `hydralisk-lurker-fix-20261009`입니다. `hydralisk-lurker-native-tests.log`는 공개 native 자체 검사 414개 통과·외부 이미지 2개 미실행 기록입니다. `hydralisk-lurker-source-build-match.json`은 Rust 27개와 Cargo/build 입력 3개, 총 30개의 일치를 기록합니다. 루트 `STATUS.json`과 `TEST-REPORT.md`는 이번 수정본에 대응합니다. 실제 게임의 러커 전체 변태 확인은 아직 남아 있습니다.

관리 소스·바이너리는 변경하지 않았습니다. `building-production-managed-tests.log`는 2026-10-07에 실행한 177 + 79 + 118 = 374개 과거 결과입니다. 이번에는 관리 검사를 재실행하지 않았습니다. STATUS의 합계 788은 현재 Native 414와 이 과거 관리 374를 합산한 값이며 재사용 여부와 실행 날짜를 별도로 표시합니다.

`building-production-child-native-tests.log`, `building-production-child-source-build-match.json`, `building-production-child-report-20261007.md`, `building-production-child-status-20261007.json`은 직전 자식 버튼 관찰 수정본의 기록입니다. 당시 Native 396개가 통과했고 공개 입력 29개가 일치했습니다. 해당 기록을 이번 러커 변태 실게임 검증으로 해석하지 않습니다.
`building-production-native-tests.log`, `building-production-source-build-match.json`, `building-production-click-report-20261007.md`, `building-production-click-status-20261007.json`은 상위 버튼 이벤트 수정본의 과거 기록입니다. 당시 Native 371개가 통과했지만 이후 사용자 확인에서 배럭 3개 중 원래 선택한 한 건물만 생산했습니다. 이번에는 자식 콜백 연결과 실제 관찰 소유 여부 판정을 추가했습니다.
`nonhost-*` 기록과 `nonhost-control-report-20261007.md`, `nonhost-control-status-20261007.json`은 직전 비방장 게임 판정 수정본의 과거 기록입니다. 당시 native 363 + managed 374 = 737개가 통과했고 실제 비방장 게임 확인은 남아 있었습니다.

그 밖의 기존 로그, `public-native-tests.log`, `native-source-build-match.json`, `managed-source-build-match.json`, `game-session-reconnect-report-20261007.md`, `game-session-reconnect-status-20261007.json`은 이전 재연결 배포본의 과거 기록입니다. 과거 원본 배포 검사 수는 native 361 + managed 374 = 735개이고, 당시 공개 소스는 외부 자료 검사 3개를 제외해 native 358 + managed 374 = 732개였습니다. 과거 보고서의 실게임 미검증 표기는 작성 당시 상태입니다. 이후 사용자가 두 판 연속 정상 유지를 확인했습니다.

공개 소스에는 설치 게임의 `installed-units.dat`, upstream 전체 복사 `upstream-struct-layouts.rs`, 이를 직접 포함하던 검사 3개, 로컬 설치 경로가 담긴 원본 검사 JSON을 포함하지 않습니다. 일반 자체 배열·메모리·파일 검사는 유지하며 현재 DLL은 이번 공개 소스로 빌드했습니다.

관리 실행 파일, 커넥터, 로비 DLL, bootstrap은 기존 배포 바이트 그대로입니다. 각 실행 파일 해시는 루트 `artifact-manifest.json`에 있습니다. bootstrap과 SHA256 파일은 Git에서 바이트를 변환하지 않습니다. 실행 로그와 게임 원본은 배포하지 않습니다.

`verification/AllianceTests.cs`는 별도 관리 모델 검사이며 production Native 대신 fixture stub을 사용합니다. AllianceSnapshot.cs와 AllianceOverlayForm.cs를 함께 x64, warnaserror, Drawing/Forms 참조로 컴파일합니다. 일반 관리 검사 스크립트와 별도로 118개를 검사합니다.
