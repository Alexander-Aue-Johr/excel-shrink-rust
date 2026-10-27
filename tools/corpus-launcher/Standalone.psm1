Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1')

function Assert-StandaloneLocalPath([string]$Path) {
    if ($Path -notmatch '\A[A-Za-z]:[\\/]' -or ($Path.ToCharArray() | Where-Object { [char]::IsControl($_) })) {
        throw "Standalone Windows paths must use a local drive such as C:\ or D:\: $Path"
    }
    return [IO.Path]::GetFullPath($Path)
}

function Copy-StandaloneArchive {
    param(
        [Parameter(Mandatory=$true)][string]$Source,
        [Parameter(Mandatory=$true)][string]$Destination,
        [string]$ExpectedSha256
    )
    $Destination = Assert-StandaloneLocalPath $Destination
    if ($ExpectedSha256 -and $ExpectedSha256 -notmatch '\A[0-9a-fA-F]{64}\z') { throw 'Expected archive SHA-256 must contain 64 hexadecimal characters.' }
    $sourcePath = [IO.Path]::GetFullPath($Source)
    $inputStream = $null; $outputStream = $null; $temporary = $null
    try {
        # Deny concurrent Windows writers while hashing and streaming the source.
        # The source may still be on WSL during this one deployment operation.
        $inputStream = [IO.FileStream]::new($sourcePath, [IO.FileMode]::Open, [IO.FileAccess]::Read,
            [IO.FileShare]::Read, 1MB, [IO.FileOptions]::SequentialScan)
        $hash = [Security.Cryptography.SHA256]::Create()
        try { $sourceHash = ([BitConverter]::ToString($hash.ComputeHash($inputStream))).Replace('-', '').ToLowerInvariant() }
        finally { $hash.Dispose() }
        if ($ExpectedSha256 -and $sourceHash -cne $ExpectedSha256.ToLowerInvariant()) { throw 'The source archive does not match its expected SHA-256.' }
        $bytes = $inputStream.Length
        if ([IO.File]::Exists($Destination)) {
            $existingHash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($existingHash -cne $sourceHash) { throw "Existing archive differs; refusing to overwrite: $Destination" }
            [void](Test-LocalWorkbookArchive $Destination)
            return [pscustomobject]@{ path=$Destination; sha256=$sourceHash; bytes=$bytes }
        }
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Destination))
        $temporary = $Destination + '.' + [Guid]::NewGuid().ToString('N') + '.partial'
        $outputStream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $inputStream.Position = 0
        $inputStream.CopyTo($outputStream, 1MB)
        $outputStream.Flush($true)
        $outputStream.Dispose(); $outputStream = $null
        $copiedHash = (Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($copiedHash -cne $sourceHash) { throw 'Archive copy SHA-256 verification failed.' }
        # Inspect only the central directory. Inner malformed workbooks remain
        # available for the study's normal failure handling; no member is changed.
        [void](Test-LocalWorkbookArchive $temporary)
        [IO.File]::Move($temporary, $Destination)
        $temporary = $null
        return [pscustomobject]@{ path=$Destination; sha256=$sourceHash; bytes=$bytes }
    } finally {
        if ($outputStream) { $outputStream.Dispose() }
        if ($inputStream) { $inputStream.Dispose() }
        if ($temporary -and [IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function New-StandaloneStudyConfiguration {
    param(
        [Parameter(Mandatory=$true)]$Configuration,
        [Parameter(Mandatory=$true)][string]$Repo,
        [Parameter(Mandatory=$true)][string[]]$SelectedCorpora,
        [Parameter(Mandatory=$true)][string]$ArchiveDirectory
    )
    $Repo = Assert-StandaloneLocalPath $Repo
    $ArchiveDirectory = Assert-StandaloneLocalPath $ArchiveDirectory
    $copy = $Configuration | ConvertTo-Json -Depth 40 | ConvertFrom-Json
    if ((Get-Value $copy 'version') -ne 1 -or -not (Get-Value $copy 'settings')) { throw 'Unsupported corpus launcher configuration.' }
    if (-not $SelectedCorpora.Count) { throw 'Select at least one corpus for the standalone deployment.' }
    $selected = @(); $seenNames = @{}; $seenIds = @{}
    foreach ($entry in @($copy.corpora)) {
        if ($entry.id -notmatch '\A[0-9a-f]{64}\z' -or $seenIds.ContainsKey($entry.id) -or $seenNames.ContainsKey($entry.name)) { throw 'Duplicate or invalid corpus identity in configuration.' }
        $seenIds[$entry.id] = $true; $seenNames[$entry.name] = $true
    }
    foreach ($name in $SelectedCorpora) {
        $selectedEntries = @($copy.corpora | Where-Object { $_.name -eq $name })
        if ($selectedEntries.Count -ne 1 -or $selected -contains $selectedEntries[0].name) { throw "Unknown or repeated standalone corpus: $name" }
        $selected += $selectedEntries[0].name
    }
    $rawOutput = [string]$copy.settings.output_root
    if ($rawOutput -match '\A(?:\\\\|/)') { throw 'The existing output directory must already be on a local Windows drive.' }
    $copy.settings.output_root = Assert-StandaloneLocalPath (Resolve-StudyPath $Repo $rawOutput)
    $report = Get-Value $copy.settings 'import_report'
    if ($report) {
        if ([string]$report -match '\A(?:\\\\|/)') { throw 'The import report must be local before preparing a standalone deployment.' }
        $copy.settings.import_report = Assert-StandaloneLocalPath (Resolve-StudyPath $Repo $report)
    }
    foreach ($entry in @($copy.corpora)) {
        if ($selected -contains $entry.name) {
            $archive = Join-Path $ArchiveDirectory ($entry.id.Substring(0,20) + '.zip')
            $count = Test-LocalWorkbookArchive $archive
            $expected = Get-Value $entry 'total_workbooks'
            if ($null -ne $expected -and $count -ne [long]$expected) { throw "The copied archive for $($entry.name) contains $count workbooks; expected $expected." }
            $entry | Add-Member -NotePropertyName local_archive -NotePropertyValue $archive -Force
        } else {
            # Keep report identities without probing or forwarding remote/WSL
            # paths for corpora excluded from this native deployment.
            $entry | Add-Member -NotePropertyName local_archive -NotePropertyValue $null -Force
        }
        $entry | Add-Member -NotePropertyName url -NotePropertyValue $null -Force
    }
    $copy.PSObject.Properties.Remove('wsl_workspace')
    $copy | Add-Member -NotePropertyName standalone_windows -NotePropertyValue $true -Force
    $copy | Add-Member -NotePropertyName standalone_corpora -NotePropertyValue @($selected) -Force
    $copy.settings.download_root = $ArchiveDirectory
    $copy.settings.build_release = $false
    if ($selected -notcontains $copy.settings.corpus) { $copy.settings.corpus = $selected[0] }
    return $copy
}

Export-ModuleMember -Function Copy-StandaloneArchive,New-StandaloneStudyConfiguration
