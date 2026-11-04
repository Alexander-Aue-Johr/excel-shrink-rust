<#
.SYNOPSIS
Back up and remove abandoned Excel Shrink developer add-in registrations.
.DESCRIPTION
Run between corpus runs, with Excel closed. The default is a read-only inventory.
Only GUID entries pointing into this user's es-office-capture temporary directories
qualify. Existing manifests must confirm the same add-in ID. Other registrations stay.
#>
param(
    [switch]$Apply,
    [string]$BackupPath
)
$ErrorActionPreference = 'Stop'
if (@(Get-Process -Name EXCEL -ErrorAction SilentlyContinue).Count) {
    throw 'Close Excel before inspecting abandoned capture registrations.'
}
if ($Apply -and -not $BackupPath) { throw 'Apply requires a new BackupPath.' }
if ($Apply -and (Test-Path -LiteralPath $BackupPath)) { throw 'BackupPath already exists.' }

$registryPath = 'Software\Microsoft\Office\16.0\WEF\Developer'
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($registryPath, $Apply.IsPresent)
if ($null -eq $key) { Write-Output '{"candidates":0,"removed":0}'; return }
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
$candidates = @()
$removed = 0
try {
    foreach ($name in $key.GetValueNames()) {
        $id = [Guid]::Empty
        if (-not [Guid]::TryParse($name, [ref]$id)) { continue }
        $value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($value -isnot [string] -or -not $value.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) { continue }
        $relative = $value.Substring($temporaryRoot.Length)
        if ($relative -notmatch '^es-office-capture-[0-9a-f]{32}\\[a-zA-Z0-9_.-]+-manifest\.xml$') { continue }
        $exists = [IO.File]::Exists($value)
        if ($exists) {
            $settings = [Xml.XmlReaderSettings]::new()
            $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
            $settings.XmlResolver = $null
            $reader = [Xml.XmlReader]::Create($value, $settings)
            try {
                $manifest = [Xml.XmlDocument]::new()
                $manifest.Load($reader)
            } finally { $reader.Dispose() }
            $manifestId = $manifest.SelectSingleNode('/*[local-name()="OfficeApp"]/*[local-name()="Id"]')
            if ($null -eq $manifestId -or $manifestId.InnerText -ne $id.ToString()) { continue }
        }
        $candidates += [pscustomobject]@{ id=$name; manifest=$value; kind=[int]$key.GetValueKind($name); manifest_exists=$exists }
    }
    if ($Apply) {
        if (@(Get-Process -Name EXCEL -ErrorAction SilentlyContinue).Count) { throw 'Excel started during inventory.' }
        $backup = [IO.Path]::GetFullPath($BackupPath)
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($backup))
        @{ registry_path=$registryPath; created_utc=[DateTime]::UtcNow.ToString('o'); entries=$candidates } |
            ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $backup -Encoding UTF8
        foreach ($entry in $candidates) {
            # A replacement registration must survive even if it reuses the same ID.
            if ($key.GetValue($entry.id) -ceq $entry.manifest) {
                $key.DeleteValue($entry.id, $false)
                $removed++
            }
        }
    }
} finally { $key.Dispose() }
@{ candidates=$candidates.Count; removed=$removed; backup=$BackupPath } | ConvertTo-Json -Compress
