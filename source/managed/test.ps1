$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$testRoot = Join-Path $env:TEMP ('sc-multitest-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$testExe = Join-Path $testRoot 'SnapshotTests.exe'
$sources = @('Snapshot.cs', 'TestMonitor.cs', 'Native.cs', 'OriginalFiles.cs', 'StatusText.cs', 'UnitVisuals.cs', 'SelectionHud.cs', 'UnitNames.cs', 'AllianceSnapshot.cs', 'AllianceEditsFile.cs', 'tests\SnapshotTests.cs', 'tests\SnapshotFileTests.cs', 'tests\ConnectionRetryTests.cs', 'tests\MenuWaitTests.cs', 'tests\UnitVisualTests.cs', 'tests\SelectionHudTests.cs') | ForEach-Object { Join-Path $root $_ }
& $compiler /nologo /warnaserror+ /target:exe /platform:x64 /reference:System.Drawing.dll /out:$testExe $sources
if ($LASTEXITCODE -ne 0) { throw 'Snapshot tests did not compile.' }
& $testExe
if ($LASTEXITCODE -ne 0) { throw 'Snapshot tests failed.' }
$allianceTestExe = Join-Path $testRoot 'AllianceEditsFileTests.exe'
$allianceSources = @('Snapshot.cs', 'TestMonitor.cs', 'Native.cs', 'OriginalFiles.cs', 'StatusText.cs', 'UnitVisuals.cs', 'SelectionHud.cs', 'UnitNames.cs', 'AllianceSnapshot.cs', 'AllianceEditsFile.cs', 'tests\AllianceEditsFileTests.cs') | ForEach-Object { Join-Path $root $_ }
& $compiler /nologo /warnaserror+ /target:exe /platform:x64 /reference:System.Drawing.dll /out:$allianceTestExe $allianceSources
if ($LASTEXITCODE -ne 0) { throw 'Alliance staged-file tests did not compile.' }
& $allianceTestExe
if ($LASTEXITCODE -ne 0) { throw 'Alliance staged-file tests failed.' }
