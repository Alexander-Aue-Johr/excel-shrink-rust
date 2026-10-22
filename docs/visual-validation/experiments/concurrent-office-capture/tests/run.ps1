param([Parameter(Mandatory=$true)][string]$RuntimeRoot)
$ErrorActionPreference = 'Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $RuntimeRoot 'run.ps1'),[ref]$tokens,[ref]$errors)
$find=$ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-DotNet8'},$false)
Invoke-Expression $find[0].Extent.Text
$dotnet=Get-DotNet8
$stage=Join-Path $env:TEMP ('es-parallel-tests-'+[guid]::NewGuid().ToString('N'))
try {
    [void][IO.Directory]::CreateDirectory($stage)
    Copy-Item -LiteralPath $PSScriptRoot -Destination (Join-Path $stage 'tests') -Recurse
    Copy-Item -LiteralPath (Join-Path (Split-Path $PSScriptRoot -Parent) 'host') -Destination (Join-Path $stage 'host') -Recurse
    & $dotnet build (Join-Path $stage 'tests/ProtocolTests.csproj') -c Release -o (Join-Path $stage 'bin') --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Protocol tests did not build.' }
    & $dotnet (Join-Path $stage 'bin/ProtocolTests.dll')
    if ($LASTEXITCODE -ne 0) { throw 'Protocol tests failed.' }
} finally { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
