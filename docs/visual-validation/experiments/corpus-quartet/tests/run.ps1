param([Parameter(Mandatory=$true)][string]$DotNet)
$ErrorActionPreference='Stop'
$stage=Join-Path $env:TEMP ('quartet-tests-'+[Guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $stage | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\host') -Destination (Join-Path $stage 'host') -Recurse
    Copy-Item -LiteralPath $PSScriptRoot -Destination (Join-Path $stage 'tests') -Recurse
    & $DotNet run --project (Join-Path $stage 'tests\QuartetTests.csproj')
    if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
} finally {
    $resolved = [IO.Path]::GetFullPath($stage)
    $temporary = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
    if ([IO.Path]::GetDirectoryName($resolved) -ne $temporary -or
        [IO.Path]::GetFileName($resolved) -notmatch '^quartet-tests-[0-9a-f]{32}$') {
        throw 'Test cleanup target is outside the allocated temporary directory.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
}
