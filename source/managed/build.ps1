$ErrorActionPreference = 'Stop'
$scAllianceSourceRoot = $PSScriptRoot
$scAllianceCompiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$scAllianceSources = @(Get-ChildItem -LiteralPath $scAllianceSourceRoot -Filter '*.cs' | ForEach-Object { $_.FullName })
$scAllianceOutput = Join-Path $scAllianceSourceRoot 'SC-Launcher.exe'
$scAllianceManifest = Join-Path $scAllianceSourceRoot 'app.manifest'
& $scAllianceCompiler /nologo /warnaserror+ /target:winexe /platform:x64 /main:ScMultiTest.Program /optimize+ /win32manifest:$scAllianceManifest /reference:System.dll /reference:System.Core.dll /reference:System.Drawing.dll /reference:System.Windows.Forms.dll /out:$scAllianceOutput $scAllianceSources
if ($LASTEXITCODE -ne 0) { throw 'Launcher compilation failed.' }
Get-FileHash -LiteralPath $scAllianceOutput -Algorithm SHA256
