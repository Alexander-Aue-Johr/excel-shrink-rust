Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Value($Object, [string]$Name, $Default = $null) {
    if ($null -eq $Object) { return $Default }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $Default }
    return $property.Value
}

function Get-TextHash([string]$Text) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)))).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Resolve-StudyPath([string]$Repo, [string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'A path is required.' }
    if ($Path.ToCharArray() | Where-Object { [char]::IsControl($_) }) { throw 'A path contains a control character.' }
    if ([IO.Path]::IsPathRooted($Path)) { return [IO.Path]::GetFullPath($Path) }
    return [IO.Path]::GetFullPath((Join-Path $Repo $Path))
}

function Read-StudyJson([string]$Path) {
    if (-not [IO.File]::Exists($Path)) { return $null }
    return [IO.File]::ReadAllText($Path) | ConvertFrom-Json
}

function Save-StudyJson([string]$Path, $Value) {
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.pending'
    try {
        [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($Path)) { [IO.File]::Replace($temporary, $Path, [NullString]::Value) }
        else { [IO.File]::Move($temporary, $Path) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}

function Test-LocalWorkbookArchive([string]$Path) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if (-not [IO.File]::Exists($Path)) { throw "ZIP file not found: $Path" }
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $count = @($zip.Entries | Where-Object { $_.FullName -match '(?i)\.(xlsx|xls)$' -and -not $_.FullName.EndsWith('/') }).Count
        if ($count -eq 0) { throw "The ZIP contains no XLSX or XLS workbooks: $Path" }
        return $count
    } finally { $zip.Dispose() }
}

function Set-StudyArchiveBinding($Entry, [string]$Path, [string]$Repo, [string]$OutputRoot = '.tmp/regression/study') {
    $Path=Resolve-StudyPath $Repo $Path
    $count=Test-LocalWorkbookArchive $Path
    $expected=Get-Value $Entry 'total_workbooks'
    if($null -ne $expected -and $count -ne [int]$expected){throw "This ZIP contains $count workbooks; expected $expected. Add a different corpus separately."}
    $oldPath=Get-Value $Entry 'local_archive'
    if($oldPath){$oldPath=Resolve-StudyPath $Repo $oldPath}
    if($oldPath -ne $Path){
        # An explicit rebinding is rare and may reuse thousands of results.
        # Compare the archives once; routine launches keep the original path
        # and leave the runner's source-catalog validation authoritative.
        $knownHash=Get-Value $Entry 'archive_sha256'
        if($oldPath -and [IO.File]::Exists($oldPath)){$knownHash=(Get-FileHash -LiteralPath $oldPath -Algorithm SHA256).Hash.ToLowerInvariant()}
        $records=Join-Path (Resolve-StudyPath $Repo $OutputRoot) 'records'
        if(-not $knownHash -and [IO.Directory]::Exists($records) -and [IO.Directory]::GetFiles($records,'*.json').Length -gt 0){
            throw 'The previous ZIP is missing and no verified archive hash is available. This binding cannot be safely checked against existing results. Restore the original archive at its previous location or add a new corpus.'
        }
        $newHash=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
        if($knownHash -and $knownHash -ne $newHash){throw 'This ZIP does not match the previously linked archive. Add a different corpus separately.'}
        $Entry | Add-Member -NotePropertyName archive_sha256 -NotePropertyValue $newHash -Force
    }
    $Entry.local_archive=$Path
}

function Get-DefaultStudySettings {
    # Conservative defaults are only caps, not reservations. Existing settings
    # from this machine take precedence during migration.
    $ram = 8GB
    try { $ram = [UInt64](Get-CimInstance Win32_ComputerSystem -ErrorAction Stop).TotalPhysicalMemory } catch { }
    $small = $ram -le 9GB
    return [pscustomobject][ordered]@{
        machine_name = [Environment]::MachineName
        machine_label = [Environment]::MachineName
        corpus = 'Data.gov'
        output_root = '.tmp/regression/study'
        download_root = '.tmp/archive-regression/downloads'
        comparison_groups = 1
        control_room_port = 8771
        partitions = 4
        partition = 0
        order = 'size-desc'
        hide_excel = $false
        targeted_comparison = $true
        targeted_skip_check_performance = $true
        chart_capture_mode = 'deterministic'
        targeted_check_performance = $false
        worker_memory_mib = $(if ($small) { 2048 } else { 4096 })
        excel_memory_mib = $(if ($small) { 2048 } else { 4096 })
        resident_memory_mib = $(if ($small) { 4096 } else { 16384 })
        system_reserve_mib = 0
        open_timeout_seconds = 600
        job_timeout_seconds = 1800
        tall_sheet_threshold = 40
        tall_sheet_edge_tiles = 2
        export_results = $true
        build_release = $true
        reuse_prepared_from = @()
        import_report = $null
    }
}

function Import-StudyConfiguration {
    param([string]$Repo, [string]$ConfigurationPath, [string]$CatalogPath = (Join-Path $PSScriptRoot 'catalog.json'))
    $saved = Read-StudyJson $ConfigurationPath
    if ($saved) {
        if ((Get-Value $saved 'version') -ne 1) { throw 'Unsupported corpus launcher configuration version.' }
        # New control defaults do not change corpus IDs, archive locations or
        # prior results. Old machine configurations acquire only these defaults.
        foreach ($setting in @{ tall_sheet_threshold=40; tall_sheet_edge_tiles=2; system_reserve_mib=0; targeted_comparison=$true; targeted_skip_check_performance=$true; targeted_check_performance=$false }.GetEnumerator()) {
            if ($null -eq (Get-Value $saved.settings $setting.Key)) {
                $saved.settings | Add-Member -NotePropertyName $setting.Key -NotePropertyValue $setting.Value -Force
            }
        }
        # A copied local config must not impose a tower's memory caps on a laptop.
        if ((Get-Value $saved.settings 'machine_name') -ne [Environment]::MachineName) {
            $defaults = Get-DefaultStudySettings
            foreach ($name in @('machine_name','machine_label','comparison_groups','worker_memory_mib','excel_memory_mib','resident_memory_mib','system_reserve_mib')) {
                $saved.settings | Add-Member -NotePropertyName $name -NotePropertyValue $defaults.$name -Force
            }
        }
        return $saved
    }
    $catalog = Read-StudyJson $CatalogPath
    if (-not $catalog -or $catalog.version -ne 1) { throw 'Missing or unsupported corpus catalog.' }
    $settings = Get-DefaultStudySettings
    $metadataPath = Join-Path $Repo '.tmp/archive-regression/tower-active-run.json'
    $metadata = Read-StudyJson $metadataPath
    if ($metadata) {
        $settings.output_root = Get-Value $metadata 'OutputRoot' $settings.output_root
        $settings.corpus = Get-Value $metadata 'RunCorpus' $settings.corpus
        $settings.reuse_prepared_from = @(Get-Value $metadata 'ReusePreparedFrom' @())
        # This is a recorded compatibility allowlist. The previous executable
        # hash itself is deliberately NOT automatically approved for a new build.
        if ((Get-Value $metadata 'MachineLabel') -eq [Environment]::MachineName) {
            $mapping = @{ ComparisonWorkers='comparison_groups'; WorkerMemoryMiB='worker_memory_mib'; ComMemoryMiB='excel_memory_mib'; ResidentMemoryMiB='resident_memory_mib'; MachineLabel='machine_label'; ProcessingOrder='order'; HideExcel='hide_excel' }
            foreach ($name in $mapping.Keys) {
                $value = Get-Value $metadata $name
                if ($null -ne $value) { $settings.($mapping[$name]) = $value }
            }
        }
        $arguments = @(Get-Value $metadata 'Arguments' @())
        $argumentSettings=@{
            '--run-corpus'='corpus'; '--output-root'='output_root'; '--download-root'='download_root';
            '--order'='order'; '--startup-timeout-seconds'='open_timeout_seconds';
            '--workbook-timeout-seconds'='job_timeout_seconds'; '--import-report'='import_report';
            '--control-room-port'='control_room_port'
        }
        for($at=0;$at -lt $arguments.Count;$at++){
            $option=[string]$arguments[$at]
            if($option -eq '--hide-excel'){$settings.hide_excel=$true;continue}
            if($at+1 -ge $arguments.Count){continue}
            $value=$arguments[$at+1]
            if($argumentSettings.ContainsKey($option)){$settings.($argumentSettings[$option])=$value}
            elseif($option -eq '--size-part'){
                if($value -notmatch '\A([1-9][0-9]*)/([1-9][0-9]*)\z'){throw 'Invalid size partition in the previous launch metadata.'}
                $settings.partition=[int]$Matches[1];$settings.partitions=[int]$Matches[2]
            }
            elseif($option -eq '--reuse-prepared-from'){$settings.reuse_prepared_from=@(@($settings.reuse_prepared_from)+$value | Select-Object -Unique)}
            elseif($option -eq '--comparison-workers' -and (Get-Value $metadata 'MachineLabel') -eq [Environment]::MachineName){$settings.comparison_groups=$value}
        }
    }
    $config = [pscustomobject][ordered]@{ version=1; corpora=@(); settings=$settings; access_url=$null; migration=[pscustomobject]@{ source_files=@(); archives_moved=0; downloads_started=0 } }
    foreach ($item in $catalog.corpora) {
        $config.corpora += [pscustomobject][ordered]@{
            name=$item.name; id=$item.id; url=(Get-Value $item 'url'); total_workbooks=(Get-Value $item 'total_workbooks');
            local_archive=(Join-Path (Resolve-StudyPath $Repo $settings.download_root) (Get-Value $item 'cache_file' ($item.id.Substring(0,20)+'.zip')))
        }
    }
    # Read the original sharing request first, then the portable local binding.
    # URLs remain in this ignored machine file; the committed catalog has none.
    foreach ($relative in @('.tmp/archive-regression/study-request.json','.tmp/distributed-study-request.json')) {
        $path = Join-Path $Repo $relative
        $request = Read-StudyJson $path
        if (-not $request) { continue }
        $config.migration.source_files += $path
        $access = Get-Value $request 'access_url'
        if ($access) { $config.access_url = $access }
        foreach ($source in @(Get-Value $request 'corpora' @())) {
            $id = Get-Value $source 'id' (Get-TextHash ([string]$source.url))
            $existing = @($config.corpora | Where-Object { $_.id -eq $id })
            if ($existing.Count -eq 0) {
                if (@($config.corpora | Where-Object { $_.name -eq $source.name }).Count) { throw "Conflicting corpus identity for $($source.name). Keep the existing request unchanged and resolve the mapping." }
                $entry = [pscustomobject]@{ name=$source.name; id=$id; url=$null; local_archive=$null; total_workbooks=(Get-Value $source 'total_workbooks') }
                $config.corpora += $entry
            } else { $entry = $existing[0] }
            if ($source.url -match '^https?://') {
                $entry.url = $source.url
                $candidate = Join-Path (Resolve-StudyPath $Repo $settings.download_root) ((Get-TextHash $source.url).Substring(0,20)+'.zip')
                if ([IO.File]::Exists($candidate)) { $entry.local_archive = $candidate }
            } else { $entry.local_archive = Resolve-StudyPath $Repo $source.url }
            $total = Get-Value $source 'total_workbooks'
            if ($null -ne $total) { $entry.total_workbooks = $total }
        }
    }
    return $config
}

function Add-StudyCorpus {
    param($Configuration, [string]$Name, [string]$Source, [string]$Repo)
    $Name = $Name.Trim(); $Source = $Source.Trim()
    if (-not $Name -or $Name.Length -gt 120 -or ($Name.ToCharArray() | Where-Object { [char]::IsControl($_) })) { throw 'Enter a valid name (up to 120 characters).' }
    if (@($Configuration.corpora | Where-Object { $_.name -eq $Name }).Count) { throw 'This corpus name already exists.' }
    $url = $null; $local = $null; $total = $null
    if ($Source -match '^https?://') {
        $uri = $null
        if (-not [Uri]::TryCreate($Source, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -notin @('http','https') -or $uri.UserInfo) { throw 'Enter an HTTP(S) URL without embedded credentials.' }
        if ($Source.ToCharArray() | Where-Object { [char]::IsControl($_) }) { throw 'The URL contains control characters.' }
        $url = $Source; $id = Get-TextHash $url
        # A URL may redirect or end in a signed download query. The runner checks
        # that the actual response is a ZIP before any workbook processing.
    } else {
        $local = Resolve-StudyPath $Repo $Source
        $total = Test-LocalWorkbookArchive $local
        $id = Get-TextHash ('local-corpus:' + [Guid]::NewGuid().ToString('N'))
    }
    if (@($Configuration.corpora | Where-Object { $_.id -eq $id }).Count) { throw 'This source already exists in the catalog.' }
    $item = [pscustomobject][ordered]@{ name=$Name; id=$id; url=$url; local_archive=$local; total_workbooks=$total }
    $Configuration.corpora += $item
    return $item
}

function ConvertTo-StudyRequest {
    param($Configuration, [string]$Repo, [string]$SelectedCorpus)
    $sources = @(); $found = $false; $ids = @{}; $names = @{}
    foreach ($entry in $Configuration.corpora) {
        if ($entry.id -notmatch '\A[0-9a-f]{64}\z' -or $ids.ContainsKey($entry.id) -or $names.ContainsKey($entry.name)) { throw 'Duplicate or invalid corpus identity in configuration.' }
        $ids[$entry.id]=$true; $names[$entry.name]=$true
        $local = Get-Value $entry 'local_archive'
        $url = Get-Value $entry 'url'
        if ($local) { $local = Resolve-StudyPath $Repo $local }
        if ($local -and [IO.File]::Exists($local)) { $locator = $local }
        elseif ($url) { $locator = $url }
        else { $locator = Join-Path (Resolve-StudyPath $Repo $Configuration.settings.download_root) ($entry.id.Substring(0,20)+'.zip') }
        if ($entry.name -eq $SelectedCorpus) {
            $found = $true
            if ($locator -notmatch '^https?://') { [void](Test-LocalWorkbookArchive $locator) }
        }
        $sources += [pscustomobject][ordered]@{ name=$entry.name; id=$entry.id; url=$locator; total_workbooks=(Get-Value $entry 'total_workbooks') }
    }
    if (-not $found) { throw "Unknown corpus: $SelectedCorpus" }
    $result = [ordered]@{ corpora=$sources }
    if (Get-Value $Configuration 'access_url') { $result.access_url=$Configuration.access_url }
    return [pscustomobject]$result
}

function Assert-StudySettings($Settings) {
    foreach ($rule in @(@('comparison_groups',0,2147483647),@('control_room_port',1024,65535),@('partitions',1,1000),@('partition',0,1000),@('worker_memory_mib',512,65536),@('excel_memory_mib',512,65536),@('resident_memory_mib',1024,65536),@('system_reserve_mib',0,65536),@('open_timeout_seconds',10,600),@('job_timeout_seconds',1,2147483647),@('tall_sheet_threshold',1,2147483647),@('tall_sheet_edge_tiles',1,2147483647))) {
        $number=0L; $value=[string](Get-Value $Settings $rule[0])
        if (-not [Int64]::TryParse($value,[ref]$number) -or $number -lt $rule[1] -or $number -gt $rule[2]) { throw "Invalid value for $($rule[0]): $value" }
    }
    if ([int]$Settings.partition -gt [int]$Settings.partitions) { throw 'The selected partition exceeds the partition count.' }
    if ($Settings.order -notin @('priority','size-asc','size-desc')) { throw 'Order must be priority, size-asc or size-desc.' }
    if ([string]::IsNullOrWhiteSpace($Settings.machine_label) -or ($Settings.machine_label.ToCharArray() | Where-Object { [char]::IsControl($_) })) { throw 'A valid machine name is required.' }
    foreach ($hash in @($Settings.reuse_prepared_from)) { if ($hash -notmatch '\A[0-9a-f]{64}\z') { throw 'Invalid approved build hash.' } }
    $screenProfile = Get-Value $Settings 'preparation_hash_profile' $null
    if ($screenProfile -and $screenProfile -notin @('default','maximum-cleaning','too-aggressive-cleaning')) {
        throw "Invalid preparation hash profile: $screenProfile"
    }
    if ($screenProfile -and (Get-Value $Settings 'aggressive_first' $false)) {
        throw 'Choose one screening strategy.'
    }
    if ($screenProfile -and (Get-Value $Settings 'full_scan_deferred' $false)) {
        throw 'Preparation hash screening cannot run with a deferred full scan.'
    }
}

function ConvertTo-WindowsArgument([AllowEmptyString()][string]$Argument) {
    if ($Argument.ToCharArray() | Where-Object { [char]::IsControl($_) }) { throw 'A command-line argument contains a control character.' }
    # Windows CRT quoting. No cmd.exe, Invoke-Expression or PowerShell evaluation.
    $quoted = [regex]::Replace($Argument, '(\\*)"', '$1$1\"')
    $quoted = [regex]::Replace($quoted, '(\\+)$', '$1$1')
    return '"' + $quoted + '"'
}

function New-StudyLaunchPlan {
    param($Configuration, [string]$Repo, [string]$Executable, [string]$RunDirectory, [string[]]$ImportResults = @())
    $settings=$Configuration.settings; Assert-StudySettings $settings
    $request=ConvertTo-StudyRequest $Configuration $Repo $settings.corpus
    $output=Resolve-StudyPath $Repo $settings.output_root
    $requestPath=Join-Path $RunDirectory 'request.json'
    $argsList=@('regression','study','--request-file',$requestPath,'--output-root',$output,'--download-root',(Resolve-StudyPath $Repo $settings.download_root),
        '--run-corpus',$settings.corpus,'--machine-label',$settings.machine_label,'--resident-capture','--control-room','--control-room-port',[string]$settings.control_room_port,
        '--comparison-workers',[string]$settings.comparison_groups,'--order',$settings.order,'--startup-timeout-seconds',[string]$settings.open_timeout_seconds,'--workbook-timeout-seconds',[string]$settings.job_timeout_seconds)
    if ([int]$settings.partition -gt 0) { $argsList += @('--size-part',"$($settings.partition)/$($settings.partitions)") }
    if (Get-Value $settings 'targeted_comparison' $true) {
        $argsList += '--targeted-comparison'
        $argsList += @('--chart-capture-mode', [string](Get-Value $settings 'chart_capture_mode' 'deterministic'))
        if (Get-Value $settings 'targeted_check_performance' $false) {
            $argsList += '--targeted-check-performance'
        } else {
            $argsList += '--targeted-skip-check-performance'
        }
    } else {
        if (Get-Value $settings 'full_scan_deferred' $false) { $argsList += @('--compare-only', '--full-scan-deferred') }
        if (Get-Value $settings 'aggressive_first' $false) { $argsList += '--aggressive-first' }
        $screenProfile = Get-Value $settings 'preparation_hash_profile' $null
        if ($screenProfile) { $argsList += @('--preparation-hash-profile', [string]$screenProfile) }
    }
    if (Get-Value $settings 'largest_reduction_first' $false) { $argsList += '--largest-reduction-first' }
    if ($settings.hide_excel) { $argsList += '--hide-excel' }
    foreach ($hash in @($settings.reuse_prepared_from | Select-Object -Unique)) { $argsList += @('--reuse-prepared-from',$hash) }
    $workbookList = Get-Value $settings 'workbook_list' $null
    if ($workbookList) { $argsList += @('--workbook-list',(Resolve-StudyPath $Repo $workbookList)) }
    if ($settings.import_report) { $argsList += @('--import-report',(Resolve-StudyPath $Repo $settings.import_report)) }
    foreach ($package in $ImportResults) { $argsList += @('--import-results',(Resolve-StudyPath $Repo $package)) }
    $export=$null
    if ($settings.export_results) {
        $label=($settings.machine_label+'-'+$settings.corpus) -replace '[^a-zA-Z0-9._-]','_'
        $part=if([int]$settings.partition -eq 0){'all'}else{"$($settings.partition)-of-$($settings.partitions)"}
        $export=Join-Path $output ('exports/'+$label+'-'+$part+'-'+[IO.Path]::GetFileName($RunDirectory)+'.zip')
        $argsList += @('--export-results',$export)
    }
    $environment=[ordered]@{
        EXCEL_SHRINK_REPOSITORY_ROOT=[IO.Path]::GetFullPath($Repo)
        EXCEL_SHRINK_WORKER_MEMORY_MIB=[string]$settings.worker_memory_mib
        EXCEL_SHRINK_EXCEL_MEMORY_MIB=[string]$settings.excel_memory_mib
        EXCEL_SHRINK_RESIDENT_MEMORY_MIB=[string]$settings.resident_memory_mib
        EXCEL_SHRINK_SYSTEM_RESERVE_MIB=[string]$settings.system_reserve_mib
        EXCEL_SHRINK_RESIDENT_VISIBLE='0'
        EXCEL_SHRINK_TALL_SHEET_THRESHOLD=[string]$settings.tall_sheet_threshold
        EXCEL_SHRINK_TALL_SHEET_EDGE_TILES=[string]$settings.tall_sheet_edge_tiles
    }
    return [pscustomobject]@{ Executable=(Resolve-StudyPath $Repo $Executable); Arguments=$argsList; CommandLine=(($argsList | ForEach-Object { ConvertTo-WindowsArgument $_ }) -join ' ');
        Request=$request; RequestFile=$requestPath; OutputRoot=$output; RunDirectory=$RunDirectory; Environment=$environment; ExportResults=$export;
        Observatory="http://127.0.0.1:$($settings.control_room_port)/"; Stdout=(Join-Path $RunDirectory 'stdout.log'); Stderr=(Join-Path $RunDirectory 'stderr.log') }
}

function Test-StudyOutputBusy([string]$OutputRoot) {
    $path=Join-Path $OutputRoot '.run.lock'
    if (-not [IO.File]::Exists($path)) { return $false }
    $stream=$null; $locked=$false
    try {
        $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
        $stream.Lock(0,1); $locked=$true
        return $false
    } catch [IO.IOException] {
        # Only sharing/lock violations prove another process owns the output.
        # Windows cannot byte-lock files on the WSL UNC filesystem; treating
        # ERROR_INVALID_FUNCTION as contention falsely reports an active run.
        $nativeCode=$_.Exception.GetBaseException().HResult -band 0xffff
        if ($nativeCode -in @(32,33)) { return $true }
        throw "Could not check the lock in output directory '$OutputRoot' (Windows error $nativeCode). Windows runs require an output directory that supports Windows file locks, such as C:\ or D:\ (Advanced settings > Output directory). Existing results are retained."
    }
    finally { if($stream){if($locked){$stream.Unlock(0,1)};$stream.Dispose()} }
}

function Get-StudyLaunchSummary($Plan, $Configuration) {
    return [pscustomobject][ordered]@{ corpus=$Configuration.settings.corpus; corpus_count=$Plan.Request.corpora.Count; partition=$(if($Configuration.settings.partition -eq 0){'all'}else{"$($Configuration.settings.partition)/$($Configuration.settings.partitions)"}); order=$Configuration.settings.order;
        comparison_groups=$Configuration.settings.comparison_groups; tall_sheet_threshold=$Configuration.settings.tall_sheet_threshold; tall_sheet_edge_tiles=$Configuration.settings.tall_sheet_edge_tiles; output_root=$Plan.OutputRoot; observatory=$Plan.Observatory; export_results=$Plan.ExportResults;
        archives=@($Plan.Request.corpora | ForEach-Object { [pscustomobject]@{ name=$_.name; id=$_.id; local_archive=$(if($_.url -notmatch '^https?://'){$_.url}else{$null}); available_locally=($_.url -notmatch '^https?://' -and [IO.File]::Exists($_.url)) } });
        archives_moved=0; downloads_started=0; resume_preserved=$true; approved_preparation_builds=@($Configuration.settings.reuse_prepared_from).Count }
}

Export-ModuleMember -Function Get-Value,Get-TextHash,Resolve-StudyPath,Read-StudyJson,Save-StudyJson,Test-LocalWorkbookArchive,Set-StudyArchiveBinding,Import-StudyConfiguration,Add-StudyCorpus,ConvertTo-StudyRequest,Assert-StudySettings,ConvertTo-WindowsArgument,New-StudyLaunchPlan,Test-StudyOutputBusy,Get-StudyLaunchSummary
