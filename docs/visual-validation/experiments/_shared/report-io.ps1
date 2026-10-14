# Report post-processing IO only. Capture transports are unchanged.
$script:VVReportIOSource=Join-Path $PSScriptRoot 'report-io.cs'
function Initialize-VVReportIO {
    if($null -eq ('VVReportIO' -as [type])) {
        Add-Type -Path $script:VVReportIOSource -ReferencedAssemblies @('System.dll') -ErrorAction Stop
    }
}
function Get-VVChildItem {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$LiteralPath,[switch]$Recurse,
          [switch]$File,[switch]$Directory,[string]$Filter='*',[switch]$Force)
    Initialize-VVReportIO
    $stack=[Collections.Generic.Stack[string]]::new()
    $stack.Push([VVReportIO]::FullPath($LiteralPath))
    while($stack.Count -gt 0) {
        foreach($entry in [VVReportIO]::Children($stack.Pop())) {
            # Return links as entries so callers can report/skip them, but never traverse them.
            if($Recurse -and $entry.PSIsContainer -and -not($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                $stack.Push($entry.FullName)
            }
            if($File -and $entry.PSIsContainer){continue}
            if($Directory -and -not $entry.PSIsContainer){continue}
            if($entry.Name -like $Filter){$entry}
        }
    }
}
function Add-VVZipFile($Zip,[string]$Path,[string]$Relative,$Compression) {
    Initialize-VVReportIO
    $inputStream=$null;$output=$null
    try {
        $inputStream=[VVReportIO]::OpenRead($Path)
        $entry=$Zip.CreateEntry($Relative,$Compression)
        $output=$entry.Open()
        $copied=[VVReportIO]::CopyAndHash($inputStream,$output)
        return [ordered]@{path=$Relative;stored_bytes=$copied.Length;sha256=$copied.Sha256}
    } finally {
        if($null -ne $output){$output.Dispose()}
        if($null -ne $inputStream){$inputStream.Dispose()}
    }
}
function Add-VVZipText($Zip,[string]$Relative,[string]$Text) {
    $bytes=[Text.Encoding]::UTF8.GetBytes($Text)
    $inputStream=[IO.MemoryStream]::new($bytes,$false);$output=$null
    try {
        $entry=$Zip.CreateEntry($Relative,[IO.Compression.CompressionLevel]::Optimal)
        $output=$entry.Open()
        $copied=[VVReportIO]::CopyAndHash($inputStream,$output)
        return [ordered]@{path=$Relative;stored_bytes=$copied.Length;sha256=$copied.Sha256}
    } finally {if($null -ne $output){$output.Dispose()};$inputStream.Dispose()}
}
