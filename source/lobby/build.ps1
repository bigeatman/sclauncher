param([string]$OutputDirectory=(Join-Path $PSScriptRoot '../../artifacts/bin'))
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$scCompiler=(Get-Command gcc -ErrorAction Stop).Source
$scOutput=Join-Path $OutputDirectory 'sc_lobby_ui.dll'
& $scCompiler -shared -O1 -g -fno-omit-frame-pointer -Wall -Wextra -Werror -static-libgcc -ffreestanding -fno-builtin -nostdlib '-Wl,--entry,DllMain' (Join-Path $PSScriptRoot 'sc_lobby_ui.c') -lkernel32 -lmsvcrt -o $scOutput
if($LASTEXITCODE -ne 0){throw 'Lobby module compilation failed'}
$scHash=(Get-FileHash -LiteralPath $scOutput -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($scOutput+'.sha256',$scHash+"`n",[Text.UTF8Encoding]::new($false))
Get-FileHash -LiteralPath $scOutput -Algorithm SHA256
