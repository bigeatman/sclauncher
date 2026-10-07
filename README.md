# SC Launcher

StarCraft 1.23.10.13515 x64용 Windows 런처입니다.

최신 배포본: **building-production-click-fix-20261007**.

백틱으로 같은 종류의 건물 전체 제어를 켠 뒤 생산 버튼을 누르면 원래 선택한 건물만 생산하던 입력 경로를 보완했습니다. 본인 유닛 제어와 본인 → 컴퓨터 동맹은 방장 여부와 관계없이 사용합니다. 로비 슬롯 변경은 방장만 사용합니다.

## 실행

저장소 전체를 다운로드해 한 폴더에 둔 다음 `SC-Launcher.exe`를 실행합니다. 실행 파일 옆의 DLL, `CefConnector.exe`, `bootstrap.js`, SHA256 파일을 함께 유지합니다.

이번 새 모듈 적용 시 스타와 기존 런처를 한 번 종료하고 새 런처로 연결합니다. 이후 게임을 끝내고 다음 게임에 들어갈 때는 종료할 필요가 없습니다. 각 게임에서 본인 유닛을 새로 선택하고 백틱을 눌러 제어 그룹을 구성합니다.

직전 재연결 수정본은 사용자가 두 판 연속 기능 유지를 확인했습니다. 이번 생산 버튼 수정본과 직전 비방장 수정의 실제 게임 확인은 아직 남아 있습니다. 사용 절차는 [README.txt](README.txt), 변경 근거·검사 결과는 [TEST-REPORT.md](TEST-REPORT.md)를 참고하세요.

## 소스와 빌드

| 경로 | 내용 |
| --- | --- |
| `source/managed` | Windows 런처·오버레이·원본 검사 |
| `source/native` | 유닛 제어·동맹·게임 간 연결 관리 Rust 모듈 |
| `source/connector` | 로비 연결 보조 프로그램 |
| `source/lobby` | CEF 로비 모듈과 자체 검사 |
| `verification` | 검사 결과 및 소스 대조 기록 |
| `third-party` | 의존성 고지와 라이선스 |

Windows의 .NET Framework C# 컴파일러, Rust의 `x86_64-pc-windows-gnu` 타깃과 MinGW GCC가 필요합니다. Rust 의존성은 `Cargo.lock`에 고정되어 있습니다.

```powershell
.\build.ps1
```

결과는 `artifacts/bin`에 저장합니다. 빌드 스크립트는 스타나 런처를 실행하지 않습니다. 자세한 명령은 [source/BUILD.txt](source/BUILD.txt)를 참고하세요.

## 검사

```powershell
.\source\managed\test.ps1
cargo test --manifest-path .\source\native\Cargo.toml --target x86_64-pc-windows-gnu --release --locked
```

외부 게임 이미지 검사 2개는 기본 검사에서 실행되지 않습니다. 설치 게임 데이터, 원본 설치 파일, 사용자 실행 로그는 저장소에 포함하지 않습니다. 현재·과거 검증 기록의 구분과 별도 동맹 모델 검사는 [verification/README.md](verification/README.md), 의존성 고지는 [third-party/NOTICES.txt](third-party/NOTICES.txt)에 있습니다.
