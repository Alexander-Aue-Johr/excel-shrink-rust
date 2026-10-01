param([Parameter(Mandatory=$true)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$clsid = (Get-Item -LiteralPath 'Registry::HKEY_CLASSES_ROOT\Excel.Application\CLSID').GetValue('')
$server = (Get-Item -LiteralPath "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid\LocalServer32").GetValue('')
$match = [regex]::Match([string]$server, '(?i)^\s*"?(.+?\.exe)"?(?:\s|$)')
if (-not $match.Success) { throw "Cannot resolve Excel executable: $server" }
$exe = Get-Item -LiteralPath $match.Groups[1].Value
$graphics = [System.Drawing.Graphics]::FromHwnd([IntPtr]::Zero)
try { $dpi = @($graphics.DpiX, $graphics.DpiY) } finally { $graphics.Dispose() }
$fontRoots = @((Join-Path $env:WINDIR 'Fonts'), (Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\Fonts'))
$fonts = @(foreach ($root in $fontRoots) {
    if (Test-Path -LiteralPath $root) {
        Get-ChildItem -LiteralPath $root -File | Sort-Object Name | ForEach-Object {
            "$($_.FullName)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)"
        }
    }
})
$printer = (New-Object System.Drawing.Printing.PrinterSettings).PrinterName
$signature = [ordered]@{
    schema = 1; machine = $env:COMPUTERNAME; user = $env:USERNAME
    excel_path = $exe.FullName; excel_version = $exe.VersionInfo.FileVersion
    excel_length = $exe.Length; excel_write_ticks = $exe.LastWriteTimeUtc.Ticks
    os = [Environment]::OSVersion.VersionString; dpi = $dpi
    culture = [Globalization.CultureInfo]::CurrentCulture.Name
    ui_culture = [Globalization.CultureInfo]::CurrentUICulture.Name
    printer = $printer; fonts = $fonts
}
ConvertTo-Json -InputObject $signature -Depth 5 -Compress | Set-Content -LiteralPath $OutputPath -Encoding UTF8
