Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1')

function Invoke-StudyReleaseBuild([string]$Repo, [string]$RunDirectory, [string]$CargoExecutable='cargo') {
    $log=Join-Path $RunDirectory 'build.log'
    $stderr=Join-Path $RunDirectory 'build.stderr.log'
    # Windows PowerShell turns redirected native stderr into terminating errors
    # under ErrorActionPreference=Stop, even for successful compiler warnings.
    $build=Start-Process -FilePath $CargoExecutable -ArgumentList @('build','--release','-p','excel-shrink-cli','--bin','excel-shrink-cli') -WorkingDirectory $Repo -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $stderr -Wait -PassThru
    if ($build.ExitCode -ne 0) { throw "Release-Build fehlgeschlagen (Code $($build.ExitCode)). Details: $log und $stderr" }
}

function Get-ControlHostVersion([string]$Repo) {
    [string[]]$files = @(Get-ChildItem -LiteralPath (Join-Path $Repo 'excel-shrink-cli/office-capture/resident-host') -File | Where-Object { $_.Extension -in @('.cs','.csproj') } | ForEach-Object { $_.FullName })
    [Array]::Sort($files,[StringComparer]::Ordinal)
    $stream=[IO.MemoryStream]::new(); $writer=[IO.BinaryWriter]::new($stream)
    try {
        foreach($path in $files){
            $name=[Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFileName($path)); $bytes=[IO.File]::ReadAllBytes($path)
            $writer.Write([UInt64]$name.Length); $writer.Write($name); $writer.Write([UInt64]$bytes.Length); $writer.Write($bytes)
        }
        $writer.Flush(); $hash=[Security.Cryptography.SHA256]::Create()
        try { return ([BitConverter]::ToString($hash.ComputeHash($stream.ToArray()))).Replace('-','').ToLowerInvariant() }
        finally { $hash.Dispose() }
    } finally { $writer.Dispose(); $stream.Dispose() }
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
        try { $listener.Start() } catch { throw "Port $Port ist bereits belegt. Einen anderen Observatory-Port wählen." }
        finally { $listener.Stop() }
        return
    }
    $expected=Join-Path $Plan.OutputRoot 'control-room'
    if ((Get-Value $health 'kind') -ne 'excel-shrink-control-room' -or (Normalize-StudyPath $health.root) -ne (Normalize-StudyPath $expected)) { throw "Port $Port gehört zu einem anderen Lauf oder Dienst. Bitte einen anderen Port wählen." }
    if ($health.host_version -eq (Get-ControlHostVersion $Repo)) { return }
    if (Test-StudyOutputBusy $Plan.OutputRoot) { throw 'Der vorhandene Korpuslauf ist noch aktiv.' }
    # Only refresh our own idle control host. A foreign listener is never stopped.
    $connections=@(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop)
    $owners=@($connections.OwningProcess | Select-Object -Unique)
    if ($owners.Count -ne 1) { throw 'Der alte Kontrollserver konnte nicht eindeutig identifiziert werden.' }
    $owner=Get-CimInstance Win32_Process -Filter "ProcessId=$($owners[0])" -ErrorAction Stop
    $settingsPath=Join-Path $expected 'host-settings.json'
    $command=([string]$owner.CommandLine).Replace('\\?\','').Replace('/','\')
    if ($owner.Name -ne 'dotnet.exe' -or $command -notlike '*--control-room*' -or $command.IndexOf($settingsPath,[StringComparison]::OrdinalIgnoreCase) -lt 0) { throw 'Der alte Kontrollserver gehört nicht eindeutig zu diesem Ergebnisverzeichnis.' }
    $process=Get-Process -Id $owner.ProcessId -ErrorAction Stop
    if ([math]::Abs(($process.StartTime.ToUniversalTime()-$owner.CreationDate.ToUniversalTime()).TotalSeconds) -gt 1) { throw 'Die Prozessidentität des Kontrollservers hat sich geändert.' }
    $process | Stop-Process -Force -ErrorAction Stop
}

function Start-StudyLaunch {
    param($Configuration, [string]$ConfigurationPath, [string]$Repo, $Plan)
    $gate=[Threading.Mutex]::new($false,('Local\ExcelShrink-CorpusLaunch-'+(Get-TextHash $Plan.OutputRoot).Substring(0,20)))
    $owned=$false
    try {
        try { $owned=$gate.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned=$true }
        if (-not $owned -or (Test-StudyOutputBusy $Plan.OutputRoot)) { throw "Dieser Ergebnisordner wird bereits verwendet. Den aktiven Lauf im Observatory öffnen: $($Plan.Observatory)" }
        [void][IO.Directory]::CreateDirectory($Plan.RunDirectory)
        Save-StudyJson $ConfigurationPath $Configuration
        Save-StudyJson $Plan.RequestFile $Plan.Request
        if ($Configuration.settings.build_release) {
            Invoke-StudyReleaseBuild $Repo $Plan.RunDirectory
        }
        if (-not [IO.File]::Exists($Plan.Executable)) { throw "Executable fehlt. Zuerst cargo build --release -p excel-shrink-cli ausführen: $($Plan.Executable)" }
        Update-OwnedControlHost $Repo $Plan ([int]$Configuration.settings.control_room_port)
        $original=@{}
        try {
            foreach($key in $Plan.Environment.Keys){$original[$key]=[Environment]::GetEnvironmentVariable($key,'Process');[Environment]::SetEnvironmentVariable($key,$Plan.Environment[$key],'Process')}
            # Start-Process on Windows detaches the long run from the task's
            # terminal. Its stdout/stderr stay on disk when VS Code closes.
            $process=Start-Process -FilePath $Plan.Executable -ArgumentList $Plan.CommandLine -WorkingDirectory $Repo -WindowStyle Hidden -RedirectStandardOutput $Plan.Stdout -RedirectStandardError $Plan.Stderr -PassThru
        } finally { foreach($key in $original.Keys){[Environment]::SetEnvironmentVariable($key,$original[$key],'Process')} }
        $binaryHash=(Get-FileHash -LiteralPath $Plan.Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $commit=$null
        try { $commit=(& git -C $Repo rev-parse HEAD 2>$null).Trim() } catch { }
        $receipt=[pscustomobject][ordered]@{ version=1; Pid=$process.Id; StartedUtc=$process.StartTime.ToUniversalTime().ToString('o'); Executable=$Plan.Executable; BinarySha256=$binaryHash; Commit=$commit;
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
            if($process.HasExited){throw "Der Runner wurde beendet (Code $($process.ExitCode)). Details: $($Plan.Stderr)"}
            if(Test-StudyOutputBusy $Plan.OutputRoot){break}
            Start-Sleep -Milliseconds 100
        }
        return $receipt
    } finally { if($owned){$gate.ReleaseMutex()};$gate.Dispose() }
}

Export-ModuleMember -Function Start-StudyLaunch,Get-ControlHostVersion
