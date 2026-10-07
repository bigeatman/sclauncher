# 검증 기록 구분

루트 실행 파일, `STATUS.json`, `TEST-REPORT.md`, 기존 검사 로그 및 소스 대조 JSON은 `game-session-reconnect-fix-20261007` 배포본의 기록입니다. 해당 배포본은 native 361개 + managed 374개 = 735개를 통과했습니다. 소스 대조 JSON은 그때 빌드한 원본 배포 소스에 대한 해시입니다.

공개 소스에서는 다음 원본 자료와 그 자료를 포함하던 검사만 제외했습니다. 런타임 기능 코드는 변경하지 않았습니다.

- 설치 게임에서 가져온 `installed-units.dat`와 직접 DAT 대조 검사 2개.
- upstream 전체 복사 `upstream-struct-layouts.rs`와 그 파일 대조 검사 1개.
- 로컬 설치 파일 경로를 담은 `original-files-verified.json`.

일반 자체 데이터·메모리·파일 검사는 유지했습니다. 공개 소스 재검사 결과는 `public-native-tests.log`에 있습니다: native 358개 통과, 외부 이미지 검사 2개 미실행. 관리 소스와 해당 검사 자료는 배포본과 같으며 177 + 79 + 118 = 374개가 통과했습니다. 따라서 공개 소스 검사 합계는 732개입니다.

`native-source-build-match.json`은 기존 배포 기록이므로 공개 소스에서 검사 자료만 제거한 `building_commands.rs`, `runtime_tests.rs`의 전체 파일 해시와는 다릅니다. 포함된 DLL은 직전 검증된 배포 DLL을 보존합니다.

소스의 로비 모듈과 커넥터도 배포된 바이너리에 대응하는 작성본을 확인해 포함했습니다. 새 `build.ps1`은 로컬 도구의 개인 경로 대신 표준 C# 컴파일러와 PATH의 GCC/Rust 도구를 사용합니다. 빌드와 공개 검사에서는 게임 실행·연결·입력을 수행하지 않습니다.
