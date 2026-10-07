param([switch]$Offline)
$ErrorActionPreference='Stop'
$scOutput=Join-Path $PSScriptRoot 'artifacts/bin'
New-Item -ItemType Directory -Path $scOutput -Force | Out-Null
& (Join-Path $PSScriptRoot 'source/managed/build.ps1')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'source/managed/SC-Launcher.exe') -Destination $scOutput -Force
& (Join-Path $PSScriptRoot 'source/connector/build.ps1') -OutputDirectory $scOutput
& (Join-Path $PSScriptRoot 'source/lobby/build.ps1') -OutputDirectory $scOutput
$scCargo=@('build','--manifest-path',(Join-Path $PSScriptRoot 'source/native/Cargo.toml'),'--target','x86_64-pc-windows-gnu','--target-dir',(Join-Path $PSScriptRoot 'artifacts/native-target'),'--release','--locked')
if($Offline){$scCargo+='--offline'}
& cargo @scCargo
if($LASTEXITCODE -ne 0){throw 'Native module compilation failed'}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'artifacts/native-target/x86_64-pc-windows-gnu/release/sc_multi_test.dll') -Destination $scOutput -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'bootstrap.js') -Destination $scOutput -Force
foreach($scName in @('sc_multi_test.dll','sc_lobby_ui.dll','bootstrap.js')) {
 $scPath=Join-Path $scOutput $scName
 $scHash=(Get-FileHash -LiteralPath $scPath -Algorithm SHA256).Hash.ToLowerInvariant()
 [IO.File]::WriteAllText($scPath+'.sha256',$scHash+"`n",[Text.UTF8Encoding]::new($false))
}
Write-Output ('Build complete: '+$scOutput)
