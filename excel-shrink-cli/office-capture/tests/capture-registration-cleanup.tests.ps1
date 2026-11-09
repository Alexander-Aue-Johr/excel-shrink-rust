$ErrorActionPreference = 'Stop'
$source = Join-Path (Split-Path -Parent $PSScriptRoot) 'CaptureSetup.psm1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'CaptureSetup contains syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Test-MissingCaptureManifest', 'Remove-MissingCaptureRegistrations')) {
        Invoke-Expression $function.Extent.Text
    }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
$root = Join-Path ([IO.Path]::GetTempPath()) ('capture-registry-test-' + [Guid]::NewGuid().ToString('N'))
$registryPath = 'Software\SheetShrinkTests\' + [Guid]::NewGuid().ToString('N')
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($registryPath)
try {
    [void][IO.Directory]::CreateDirectory($root)
    $stage = Join-Path $root ('es-office-capture-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage)
    $missing = Join-Path $stage 'one-manifest.xml'
    $existing = Join-Path $stage 'manifest.xml'
    [IO.File]::WriteAllText($existing, 'existing')
    $outside = Join-Path $root 'unrelated.xml'
    $missingId = [Guid]::NewGuid().ToString()
    $existingId = [Guid]::NewGuid().ToString()
    $outsideId = [Guid]::NewGuid().ToString()
    $key.SetValue($missingId, $missing)
    $key.SetValue($existingId, $existing)
    $key.SetValue($outsideId, $outside)
    $key.SetValue('not-a-guid', $missing)
    $backups = Join-Path $root 'backups'
    Remove-MissingCaptureRegistrations $key @($root) $backups
    Assert ($null -eq $key.GetValue($missingId)) 'Missing owned registration survived.'
    Assert ($key.GetValue($existingId) -eq $existing) 'Existing manifest was removed.'
    Assert ($key.GetValue($outsideId) -eq $outside) 'Unrelated registration was removed.'
    Assert ($key.GetValue('not-a-guid') -eq $missing) 'Unknown ID was removed.'
    $files = @(Get-ChildItem -LiteralPath $backups -File)
    Assert ($files.Count -eq 1) 'Expected one backup.'
    $receipt = Get-Content -LiteralPath $files[0].FullName -Raw | ConvertFrom-Json
    Assert ($receipt.entries[0].id -eq $missingId) 'Backup omitted the deleted ID.'
    Assert ($receipt.entries[0].manifest -eq $missing) 'Backup omitted the original path.'
    Remove-MissingCaptureRegistrations $key @($root) $backups
    Assert (@(Get-ChildItem -LiteralPath $backups -File).Count -eq 1) 'Retry created an empty backup.'
    $key.SetValue($missingId, $missing)
    $blocked = Join-Path $root 'blocked'
    [IO.File]::WriteAllText($blocked, 'not a directory')
    $failed = $false
    try { Remove-MissingCaptureRegistrations $key @($root) $blocked } catch { $failed = $true }
    Assert $failed 'Backup failure was not reported.'
    Assert ($key.GetValue($missingId) -eq $missing) 'Backup failure still deleted the value.'
    Write-Output 'Capture registration cleanup tests passed.'
} finally {
    $key.Dispose()
    [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($registryPath, $false)
    if ([IO.Directory]::Exists($root)) { Remove-Item -LiteralPath $root -Recurse -Force }
}
