$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../ExcelDiagnostics.psm1') -Force
$testBase = if ($env:EXCEL_SHRINK_TEST_TEMP) { $env:EXCEL_SHRINK_TEST_TEMP } else { Join-Path $PSScriptRoot '../../.tmp' }
$root = [IO.Path]::GetFullPath((Join-Path $testBase ('diagnostic-compression-test-' + [Guid]::NewGuid().ToString('N') + '/Diagnostics/EXCEL')))
[void][IO.Directory]::CreateDirectory($root)
$path = Join-Path $root 'unknown-preallocated.log'
$bytes = New-Object byte[] (16 * 1024 * 1024)
$bytes[0] = 49
$bytes[1048576] = 170
$bytes[$bytes.Length - 1] = 127
[IO.File]::WriteAllBytes($path, $bytes)
[IO.File]::SetLastWriteTimeUtc($path, [DateTime]::UtcNow.AddMinutes(-5))
$hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
$length = (Get-Item -LiteralPath $path).Length
$handle = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
try {
    $result = Clear-ExcelDiagnosticLogs -Root $root
    if ($result.compressed_files -ne 0 -or ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::Compressed)) { throw 'Live delete-sharing file was compressed.' }
} finally { $handle.Dispose() }
$result = Clear-ExcelDiagnosticLogs -Root $root
if ($result.compressed_files -ne 1 -or $result.compression_saved_bytes -lt 15MB) { throw ('Preallocated file was not compressed: ' + ($result | ConvertTo-Json -Compress)) }
if ((Get-Item -LiteralPath $path).Length -ne $length -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $hash) { throw 'Compression changed logical file contents.' }
if ($result.retained_allocated_bytes -ge 1MB -or $result.retained_bytes -ne $length) { throw 'Logical and allocated sizes were not reported separately.' }

# Later ordinary and mapped writes must work without decompression by the caller.
$stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
try { $stream.Position = 100; $stream.WriteByte(91) } finally { $stream.Dispose() }
$map = [IO.MemoryMappedFiles.MemoryMappedFile]::CreateFromFile($path, [IO.FileMode]::Open, ('es-compression-test-' + [Guid]::NewGuid().ToString('N')), 0, [IO.MemoryMappedFiles.MemoryMappedFileAccess]::ReadWrite)
try {
    $view = $map.CreateViewAccessor()
    try { $view.Write([long]200, [byte]92); $view.Flush() } finally { $view.Dispose() }
} finally { $map.Dispose() }
$bytes[100] = 91
$bytes[200] = 92
$algorithm = [Security.Cryptography.SHA256]::Create()
try { $expected = ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '') } finally { $algorithm.Dispose() }
if ((Get-Item -LiteralPath $path).Length -ne $length -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $expected) { throw 'Reopened or memory-mapped writes changed other bytes.' }

$live = Join-Path $root 'live-writer-without-an-open-handle.log'
$liveBytes = New-Object byte[] (16 * 1024 * 1024)
$stamp = [DateTime]::UtcNow
$header = [Text.Encoding]::UTF8.GetBytes($stamp.ToString('MM/dd/yyyy HH:mm:ss.fff', [Globalization.CultureInfo]::InvariantCulture) + "`tEXCEL (0x" + $PID.ToString('X') + ")`t0x1`ttest`n")
[Array]::Copy($header, $liveBytes, $header.Length)
[IO.File]::WriteAllBytes($live, $liveBytes)
[IO.File]::SetCreationTimeUtc($live, $stamp)
[IO.File]::SetLastWriteTimeUtc($live, $stamp.AddMinutes(-5))
$result = Clear-ExcelDiagnosticLogs -Root $root
if ($result.compressed_files -ne 0 -or ((Get-Item -LiteralPath $live).Attributes -band [IO.FileAttributes]::Compressed)) { throw 'Known live writer log was compressed between its opens.' }

# Compression is bounded per pass; it never changes the directory default.
for ($index = 0; $index -lt 33; $index++) {
    $item = Join-Path $root ('blank-' + $index + '.log')
    [IO.File]::WriteAllBytes($item, (New-Object byte[] 65536))
    [IO.File]::SetLastWriteTimeUtc($item, [DateTime]::UtcNow.AddMinutes(-5))
}
$result = Clear-ExcelDiagnosticLogs -Root $root
if ($result.compression_attempts -ne 32 -or $result.compressed_files -ne 32) { throw 'Compression exceeded or did not fill its bounded pass budget.' }
if ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::Compressed) { throw 'Compression leaked to the directory default.' }
$result = Clear-ExcelDiagnosticLogs -Root $root -MaximumCompressionFiles 0
if ($result.compression_attempts -ne 0) { throw 'Disabled compression still attempted a file.' }
$result = Clear-ExcelDiagnosticLogs -Root $root -MaximumCompressionFiles 128
if ($result.compressed_files -ne 1) { throw 'Configured compression budget did not finish the backlog.' }
'PASS: exclusive lossless compression preserves 16 MiB and SHA256, saves >15 MiB, supports ordinary/mapped writes, skips live files and caps each pass at 32.'
