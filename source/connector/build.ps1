param([string]$OutputDirectory=(Join-Path $PSScriptRoot '../../artifacts/bin'))
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$scCompiler=Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
$scOutput=Join-Path $OutputDirectory 'CefConnector.exe'
& $scCompiler /nologo /warnaserror+ /target:exe /platform:x64 /optimize+ /reference:System.dll /reference:System.Core.dll /reference:System.Management.dll ('/out:'+$scOutput) (Join-Path $PSScriptRoot 'CefConnector.cs')
if($LASTEXITCODE -ne 0){throw 'CEF connector compilation failed'}
Get-FileHash -LiteralPath $scOutput -Algorithm SHA256
