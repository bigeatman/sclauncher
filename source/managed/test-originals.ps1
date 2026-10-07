$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$testRoot = Join-Path $env:TEMP ('sc-originals-build-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$testExe = Join-Path $testRoot 'OriginalFilesTests.exe'
& $compiler /nologo /warnaserror+ /target:exe /platform:x64 /reference:System.dll /reference:System.Core.dll /out:$testExe (Join-Path $root 'OriginalFiles.cs') (Join-Path $root 'tests\OriginalFilesTests.cs')
if ($LASTEXITCODE -ne 0) { throw 'Original-file preservation tests did not compile.' }
& $testExe
if ($LASTEXITCODE -ne 0) { throw 'Original-file preservation tests failed.' }
