$ErrorActionPreference = 'Stop'
$regressionRoot = Join-Path $PSScriptRoot '../../src/regression'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('es-progress-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    foreach ($case in @(
        @{ script = 'excel_open.ps1'; function = 'Progress'; phase = 'open' },
        @{ script = 'excel_recalculate.ps1'; function = 'Write-RecalcProgress'; phase = 'recalc' }
    )) {
        # Load only the actual writer function, without starting Excel.
        $tokens = $null
        $parseErrors = $null
        $ast = [Management.Automation.Language.Parser]::ParseFile(
            (Join-Path $regressionRoot $case.script), [ref]$tokens, [ref]$parseErrors)
        if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
        $definition = $ast.Find({ param($node)
            $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $case.function
        }, $true)
        if ($null -eq $definition) { throw "Missing writer in $($case.script)" }
        . ([ScriptBlock]::Create($definition.Extent.Text))

        $ProgressPath = Join-Path $testRoot ($case.phase + '.log')
        & $case.function 'first'
        $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
        $reader = [IO.File]::Open($ProgressPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
        try {
            # Keeping a reader open reproduces the sharing conflict deterministically.
            # Rust's default File::open uses these same Windows sharing flags.
            $message = 'gr' + [char]0x00FC + 'n'
            1..20 | ForEach-Object { & $case.function "$message $_" }
            $textReader = New-Object IO.StreamReader($reader, [Text.Encoding]::UTF8)
            try { $lines = $textReader.ReadToEnd().TrimEnd().Split([char]10) }
            finally { $textReader.Dispose() }
            if ($lines.Count -ne 21) { throw "$($case.script): missing or overwritten progress" }
            if (-not $lines[0].TrimEnd().EndsWith("$($case.phase) first")) { throw 'Initial message was lost' }
            if (-not $lines[20].TrimEnd().EndsWith("$($case.phase) $message 20")) { throw 'Appended UTF-8 message was lost' }
        } finally {
            $reader.Dispose()
            Remove-Item -LiteralPath $ProgressPath -Force
        }
    }
    Write-Host 'PASS: both Excel progress writers append UTF-8 while a polling reader holds the log open.'
} finally {
    foreach ($name in @('open.log', 'recalc.log')) {
        $path = Join-Path $testRoot $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    Remove-Item -LiteralPath $testRoot -Force
}
