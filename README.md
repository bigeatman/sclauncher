# SC Launcher

StarCraft 1.23.10.13515 x64용 Windows 런처입니다.

최신 배포본: **game-session-reconnect-fix-20261007**.

로비 슬롯 제어, 백틱으로 같은 종류의 본인 유닛 다중 제어, 생산·랠리, 선택 정보 표시, 컴퓨터에 대한 본인 방향 동맹 설정을 포함합니다. 최신 변경은 같은 스타에서 게임을 끝내고 다음 게임에 들어갈 때 기능을 다시 연결하는 처리입니다.

## 실행

저장소 전체를 다운로드해 한 폴더에 둔 다음 `SC-Launcher.exe`를 실행합니다. 실행 파일 옆의 DLL, `CefConnector.exe`, `bootstrap.js`, SHA256 파일을 함께 유지합니다.

새 버전 적용 시 스타와 이전 런처를 한 번 종료하고 연결합니다. 이후 스타와 런처를 종료하지 않고 다음 게임에 들어갈 수 있습니다. 새 게임에서 유닛을 새로 선택하고 백틱을 눌러 제어 그룹을 구성합니다.

자세한 사용 절차는 [README.txt](README.txt), 검증 결과와 실게임 확인 항목은 [TEST-REPORT.md](TEST-REPORT.md)를 참고하세요. 이번 수정본의 실제 연속 게임 검증은 아직 확인 전입니다.

## 소스와 빌드

| 경로 | 내용 |
| --- | --- |
| `source/managed` | Windows 런처·오버레이·관리 검사 |
| `source/native` | 유닛 제어·동맹·게임 간 연결 관리 Rust 모듈 |
| `source/connector` | 로비 연결 보조 프로그램 |
| `source/lobby` | CEF 로비 모듈과 자체 검사 |
| `verification` | 검사 결과 및 배포본 소스 대조 기록 |
| `third-party` | 의존성 고지와 라이선스 |

Windows의 .NET Framework C# 컴파일러, Rust의 `x86_64-pc-windows-gnu` 타깃과 MinGW GCC가 필요합니다. Rust 의존성은 `Cargo.lock`에 고정되어 있습니다. 빌드는 다음과 같이 실행합니다.

```powershell
.\build.ps1
```

결과는 `artifacts/bin`에 저장합니다. 빌드 스크립트는 스타나 런처를 실행하지 않습니다. 자세한 명령은 [source/BUILD.txt](source/BUILD.txt)를 참고하세요.

## 검사와 배포 기록

포함된 실행 파일은 직전 배포본 그대로이며 자체 검사 **735개**를 통과했습니다. 공개 소스에는 설치 게임 데이터와 upstream 전체 복사 검사 자료를 포함하지 않아, 그 자료에 의존하던 검사 3개를 제외했습니다. 공개 소스 자체 검사는 native 358개와 managed 374개, **총 732개**입니다. 제외된 자료와 공개본 검사는 [verification/README.md](verification/README.md)에 설명합니다.

```powershell
.\source\managed\test.ps1
cargo test --manifest-path .\source\native\Cargo.toml --target x86_64-pc-windows-gnu --release --locked
```

외부 게임 이미지 검사는 `ignored` 상태이며 기본 검사에서 실행되지 않습니다. 원본 게임 파일과 사용자 실행 로그는 포함하지 않습니다. 의존성의 각 라이선스는 [third-party/NOTICES.txt](third-party/NOTICES.txt)를 따릅니다.
