Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1')

function Invoke-StudyReleaseBuild([string]$Repo, [string]$RunDirectory, [string]$CargoExecutable='cargo', $Wsl=$null) {
    $log=Join-Path $RunDirectory 'build.log'
    $stderr=Join-Path $RunDirectory 'build.stderr.log'
    # Windows PowerShell turns redirected native stderr into terminating errors
    # under ErrorActionPreference=Stop, even for successful compiler warnings.
    if ($Wsl) {
        $command='exec "$HOME/.cargo/bin/cargo" build --release --target x86_64-pc-windows-gnu -p excel-shrink-cli --bin excel-shrink-cli'
        $arguments=@('-d',$Wsl.distribution,'--cd',$Wsl.linux_repo,'--exec','bash','-lc',$command)
        $commandLine=($arguments | ForEach-Object {if ($_ -match '[\s"]') {ConvertTo-WindowsArgument ([string]$_)} else {[string]$_}}) -join ' '
        $build=Start-Process -FilePath (Join-Path $env:SystemRoot 'System32/wsl.exe') -ArgumentList $commandLine -WorkingDirectory $Repo -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $stderr -Wait -PassThru
    } else {
        $build=Start-Process -FilePath $CargoExecutable -ArgumentList @('build','--release','-p','excel-shrink-cli','--bin','excel-shrink-cli') -WorkingDirectory $Repo -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $stderr -Wait -PassThru
    }
    if ($build.ExitCode -ne 0) { throw "Release build failed (code $($build.ExitCode)). Details: $log and $stderr" }
    if ($Wsl) {
        $destination=Join-Path $Repo 'target/release/excel-shrink-cli.exe'
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        [IO.File]::Copy((Join-Path $Wsl.source_repo 'target/x86_64-pc-windows-gnu/release/excel-shrink-cli.exe'),$destination,$true)
    }
}

function Get-ControlHostVersion([string]$Executable) {
    return (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Normalize-StudyPath([string]$Path) {
    if ($Path.StartsWith('\\?\UNC\')) { $Path='\\'+$Path.Substring(8) }
    elseif ($Path.StartsWith('\\?\')) { $Path=$Path.Substring(4) }
    return [IO.Path]::GetFullPath($Path).TrimEnd('\','/')
}

function Update-OwnedControlHost([string]$Repo, $Plan, [int]$Port) {
    $health=$null
    try { $health=Invoke-RestMethod ($Plan.Observatory+'health') -TimeoutSec 2 -ErrorAction Stop } catch { }
    if (-not $health) {
        $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,$Port)
        try { $listener.Start() } catch { throw "Port $Port is already in use. Choose another Observatory port." }
        finally { $listener.Stop() }
        return
    }
    $expected=Join-Path $Plan.OutputRoot 'control-room'
    if ((Get-Value $health 'kind') -ne 'excel-shrink-control-room' -or (Normalize-StudyPath $health.root) -ne (Normalize-StudyPath $expected)) { throw "Port $Port belongs to another run or service. Choose another port." }
    if ($health.host_version -eq (Get-ControlHostVersion $Plan.Executable)) { return }
    if (Test-StudyOutputBusy $Plan.OutputRoot) { throw 'The existing corpus run is still active.' }
    # Only refresh our own idle control host. A foreign listener is never stopped.
    $connections=@(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop)
    $owners=@($connections.OwningProcess | Select-Object -Unique)
    if ($owners.Count -ne 1) { throw 'The previous control server could not be uniquely identified.' }
    $owner=Get-CimInstance Win32_Process -Filter "ProcessId=$($owners[0])" -ErrorAction Stop
    $settingsPath=Join-Path $expected 'host-settings.json'
    $command=([string]$owner.CommandLine).Replace('\\?\','').Replace('/','\')
    # Accept the old host only for this one-time migration; both variants must
    # still match the exact study settings path and the listening process.
    $legacyHost = $owner.Name -eq 'dotnet.exe' -and $command -like '*--control-room*'
    $rustHost = $owner.Name -eq 'excel-shrink-cli.exe' -and $command -like '*capture-host control-room*'
    if ((-not $legacyHost -and -not $rustHost) -or
        $command.IndexOf($settingsPath, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw 'The previous control server cannot be uniquely associated with this output directory.'
    }
    $process=Get-Process -Id $owner.ProcessId -ErrorAction Stop
    if ([math]::Abs(($process.StartTime.ToUniversalTime()-$owner.CreationDate.ToUniversalTime()).TotalSeconds) -gt 1) { throw 'The control server process identity has changed.' }
    $process | Stop-Process -Force -ErrorAction Stop
}

function Start-StudyLaunch {
    param($Configuration, [string]$ConfigurationPath, [string]$Repo, $Plan)
    $gate=[Threading.Mutex]::new($false,('Local\ExcelShrink-CorpusLaunch-'+(Get-TextHash $Plan.OutputRoot).Substring(0,20)))
    $owned=$false
    try {
        try { $owned=$gate.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned=$true }
        if (-not $owned -or (Test-StudyOutputBusy $Plan.OutputRoot)) { throw "This output directory is already in use. Open the active run in the Observatory: $($Plan.Observatory)" }
        [void][IO.Directory]::CreateDirectory($Plan.RunDirectory)
        Save-StudyJson $ConfigurationPath $Configuration
        Save-StudyJson $Plan.RequestFile $Plan.Request
        if ($Configuration.settings.build_release) {
            Invoke-StudyReleaseBuild $Repo $Plan.RunDirectory 'cargo' (Get-Value $Configuration 'wsl_workspace')
        }
        if (-not [IO.File]::Exists($Plan.Executable)) { throw "Executable not found. First run cargo build --release -p excel-shrink-cli: $($Plan.Executable)" }
        Update-OwnedControlHost $Repo $Plan ([int]$Configuration.settings.control_room_port)
        $original=@{}
        try {
            foreach($key in $Plan.Environment.Keys){$original[$key]=[Environment]::GetEnvironmentVariable($key,'Process');[Environment]::SetEnvironmentVariable($key,$Plan.Environment[$key],'Process')}
            # Start-Process on Windows detaches the long run from the task's
            # terminal. Its stdout/stderr stay on disk when VS Code closes.
            $process=Start-Process -FilePath $Plan.Executable -ArgumentList $Plan.CommandLine -WorkingDirectory $Repo -WindowStyle Hidden -RedirectStandardOutput $Plan.Stdout -RedirectStandardError $Plan.Stderr -PassThru
        } finally { foreach($key in $original.Keys){[Environment]::SetEnvironmentVariable($key,$original[$key],'Process')} }
        # Keep Excel's own diagnostic logs from filling the system drive. The
        # helper exits when this exact runner process ends (PID reuse checked).
        $diagnosticCleanup = $null
        try {
            $cleanupScript = Join-Path $Repo 'tools/clear-excel-diagnostics.ps1'
            $cleanupStatus = Join-Path $Plan.RunDirectory 'excel-diagnostics-cleanup.json'
            $cleanupArguments = @('-NoProfile','-ExecutionPolicy','Bypass','-File',
                $cleanupScript,'-OwnerPid',[string]$process.Id,'-OwnerStartedTicks',
                [string]$process.StartTime.ToUniversalTime().Ticks,'-StatusPath',$cleanupStatus)
            $cleanupCommandLine = ($cleanupArguments | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' '
            $helper = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe') -ArgumentList $cleanupCommandLine -WorkingDirectory $Repo -WindowStyle Hidden -PassThru
            $diagnosticCleanup = @{Pid=$helper.Id; StatusPath=$cleanupStatus; IntervalSeconds=30; MinimumAgeSeconds=60}
        } catch { Write-Warning "Excel diagnostic cleanup could not start: $($_.Exception.Message)" }
        $binaryHash=(Get-FileHash -LiteralPath $Plan.Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $commit=$null
        try { $commit=(& git -C $Repo rev-parse HEAD 2>$null).Trim() } catch { }
        $receipt=[pscustomobject][ordered]@{ version=1; ExcelDiagnosticsCleanup=$diagnosticCleanup; Pid=$process.Id; StartedUtc=$process.StartTime.ToUniversalTime().ToString('o'); Executable=$Plan.Executable; BinarySha256=$binaryHash; Commit=$commit;
            RequestFile=$Plan.RequestFile; OutputRoot=$Plan.OutputRoot; RunCorpus=$Configuration.settings.corpus; MachineLabel=$Configuration.settings.machine_label;
            Arguments=$Plan.Arguments; Environment=$Plan.Environment; ConfigurationPath=$ConfigurationPath; Settings=$Configuration.settings;
            Stdout=$Plan.Stdout; Stderr=$Plan.Stderr; Report=(Join-Path $Plan.OutputRoot 'report.html'); ControlRoom=$Plan.Observatory; ExportResults=$Plan.ExportResults; RunMetadata=(Join-Path $Plan.RunDirectory 'launch.json') }
        Save-StudyJson $receipt.RunMetadata $receipt
        Save-StudyJson (Join-Path ([IO.Path]::GetDirectoryName($ConfigurationPath)) 'last-run.json') $receipt
        Save-StudyJson (Join-Path $Plan.OutputRoot 'launch.json') $receipt
        # Keep the launch gate until the Rust output lock is owned, or the child
        # reports an immediate failure. The Rust lock remains authoritative.
        $until=[DateTime]::UtcNow.AddSeconds(5)
        while([DateTime]::UtcNow -lt $until){
            $process.Refresh()
            if($process.HasExited){throw "The runner exited (code $($process.ExitCode)). Details: $($Plan.Stderr)"}
            if(Test-StudyOutputBusy $Plan.OutputRoot){break}
            Start-Sleep -Milliseconds 100
        }
        return $receipt
    } finally { if($owned){$gate.ReleaseMutex()};$gate.Dispose() }
}

function Get-StudyRunStatus([string]$OutputRoot) {
    $queue=Read-StudyJson (Join-Path $OutputRoot 'control-room/queue.json')
    $busy=Test-StudyOutputBusy $OutputRoot
    $process=$null
    if($queue -and (Get-Value $queue 'runner_pid') -and (Get-Value $queue 'runner_started_unix_ms')) {
        try {
            $candidate=Get-Process -Id $queue.runner_pid -ErrorAction Stop
            $started=[DateTimeOffset]::new($candidate.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
            if($candidate.ProcessName -eq 'excel-shrink-cli' -and [math]::Abs($started-[long]$queue.runner_started_unix_ms) -le 1){$process=$candidate}
        } catch [Microsoft.PowerShell.Commands.ProcessCommandException] { }
    }
    $stop=Read-StudyJson (Join-Path $OutputRoot 'control-room/stop-request.json')
    $stopping=$null -ne $stop -and $null -ne $queue -and
        (Get-Value $stop 'runner_pid') -eq (Get-Value $queue 'runner_pid') -and
        (Get-Value $stop 'runner_started_unix_ms') -eq (Get-Value $queue 'runner_started_unix_ms')
    [pscustomobject]@{Busy=$busy;Process=$process;Queue=$queue;Stopping=$stopping;Ended=(-not $busy -and $null -eq $process)}
}

function Request-StudyRunStop([string]$OutputRoot, [int]$Port) {
    $status=Get-StudyRunStatus $OutputRoot
    if(-not $status.Process -or -not $status.Busy){throw 'No verified active runner was found for this output directory.'}
    $address="http://127.0.0.1:$Port/"
    $health=Invoke-RestMethod ($address+'health') -TimeoutSec 2 -ErrorAction Stop
    if($health.kind -ne 'excel-shrink-control-room' -or (Normalize-StudyPath $health.root) -ne (Normalize-StudyPath (Join-Path $OutputRoot 'control-room'))){throw 'The Observatory belongs to another run. No stop was requested.'}
    if(Get-Value $status.Queue 'stop_supported' $false){
        Save-StudyJson (Join-Path $OutputRoot 'control-room/stop-request.json') ([pscustomobject]@{runner_pid=$status.Queue.runner_pid;runner_started_unix_ms=$status.Queue.runner_started_unix_ms})
    }
    # Also supports old runners: drain them first, then stop only their verified
    # coordinator once its own snapshot confirms that no admitted work remains.
    Invoke-RestMethod ($address+'api/control') -Method Post -ContentType 'application/json' -Body '{"target_groups":0,"paused":false}' -TimeoutSec 3 -ErrorAction Stop | Out-Null
    return $status
}

function Complete-StudyLegacyStop([string]$OutputRoot, $Requested) {
    if(Get-Value $Requested.Queue 'stop_supported' $false){return}
    $status=Get-StudyRunStatus $OutputRoot
    if(-not $status.Process){return}
    if($status.Queue.runner_pid -ne $Requested.Queue.runner_pid -or $status.Queue.runner_started_unix_ms -ne $Requested.Queue.runner_started_unix_ms){throw 'Runner identity changed; the replacement was left running.'}
    if($status.Queue.phase -ne 'paused' -or @($status.Queue.groups | Where-Object {Get-Value $_ 'active' $false}).Count -gt 0){return}
    # Get-StudyRunStatus checked the creation time and executable name. Use its
    # process object rather than resolving the PID again for termination.
    $status.Process | Stop-Process -Force -ErrorAction Stop
}

Export-ModuleMember -Function Start-StudyLaunch,Get-ControlHostVersion,Get-StudyRunStatus,Request-StudyRunStop,Complete-StudyLegacyStop
