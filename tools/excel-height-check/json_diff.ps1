# Managed by patch_excel_json_diff.py. Runtime: PowerShell + existing Rust CLI + VBA.
[CmdletBinding()]
param(
    [string]$InputPath,
    [string]$Profile = 'default',
    [switch]$NoDiff,
    [ValidateRange(1,65536)][int]$BatchRows = 8192,
    [ValidateSet('all','used-union')][string]$RowScope = 'all',
    [ValidateRange(10,86400)][int]$StallSeconds = 1800,
    [string]$HelperWorkbook,
    [string]$LinuxRepoRoot,
    [string]$Distro,
    [switch]$DeferDiff,
    [string]$ResultFile,
    [string]$CancelFile,
    [switch]$FunctionsOnly
)
Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
# Excel Lab v2: shared explicit COM dispatch and identity-checked cleanup.
. (Join-Path $PSScriptRoot '../excel-lab/ExcelInterop.ps1')
$script:Clock = [Diagnostics.Stopwatch]::StartNew()
$script:RunLog = $null
$script:ExcelState = ''
$script:OwnerToken = ''

function Write-RunLog([string]$Stage, [string]$Message) {
    $line = '[{0} +{1,7:N1}s] [{2}] {3}' -f (Get-Date -Format 'HH:mm:ss'), $script:Clock.Elapsed.TotalSeconds, $Stage, $Message
    [Console]::WriteLine($line)
    if ($null -ne $script:RunLog) { $script:RunLog.WriteLine($line) }
}
function Write-JsonFile($Value, [string]$Path) {
    [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 20), [Text.UTF8Encoding]::new($false))
}
function Get-TextSha256([string]$Text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
# Windows CreateProcess/CRT quoting, not PowerShell or cmd.exe interpolation.
function Quote-NativeArgument([AllowEmptyString()][string]$Value) {
    if ($null -eq $Value) { $Value = '' }
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}
function Quote-WslArgument([AllowEmptyString()][string]$Value) {
    # wsl.exe treats quoted switches such as "-d" as a Linux command. Keep
    # arguments without whitespace or quotes bare; quote paths when needed.
    if ($Value -and $Value -notmatch '[\s"]') { return $Value }
    return Quote-NativeArgument $Value
}
function Stop-OwnedExcel {
    Stop-LabOwnedExcel -StateFile $script:ExcelState -OwnerToken $script:OwnerToken
}

function Invoke-LoggedProcess {
    param([string]$Executable, [string[]]$Arguments, [string]$Stage,
          [int]$TimeoutSeconds = 1800, [switch]$Capture,
          [string]$WorkingDirectory = '', [hashtable]$Environment = @{}, [string]$ProgressPath = '')
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Executable
    $quote = if ([IO.Path]::GetFileName($Executable) -ieq 'wsl.exe') { ${function:Quote-WslArgument} } else { ${function:Quote-NativeArgument} }
    $psi.Arguments = (($Arguments | ForEach-Object { & $quote $_ }) -join ' ')
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $psi.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    if ($WorkingDirectory) { $psi.WorkingDirectory = $WorkingDirectory }
    foreach ($key in $Environment.Keys) { $psi.EnvironmentVariables[$key] = [string]$Environment[$key] }
    Write-RunLog $Stage ('START ' + $Executable + ' ' + $psi.Arguments)
    $p = [Diagnostics.Process]::new()
    $p.StartInfo = $psi
    $captured = [Collections.Generic.List[string]]::new()
    $started = $false
    try {
        $started = $p.Start()
        if (-not $started) { throw 'Process did not start.' }
        $out = $p.StandardOutput.ReadLineAsync()
        $err = $p.StandardError.ReadLineAsync()
        $lastOutput = [DateTime]::UtcNow
        $lastHeartbeat = $lastOutput
        $exitSeen = $null
        $cursor=New-LabProgressCursor $ProgressPath
        $lastProgressPoll=[DateTime]::UtcNow.AddSeconds(-1)
        while ($true) {
            $received = $false
            for ($j=0; $j -lt 128 -and $null -ne $out -and $out.IsCompleted; $j++) {
                $line = $out.GetAwaiter().GetResult()
                if ($null -eq $line) { $out = $null; break }
                if ($Capture) { $captured.Add($line) }
                Write-RunLog $Stage $line
                $lastOutput = [DateTime]::UtcNow; $received = $true
                $out = $p.StandardOutput.ReadLineAsync()
            }
            for ($j=0; $j -lt 128 -and $null -ne $err -and $err.IsCompleted; $j++) {
                $line = $err.GetAwaiter().GetResult()
                if ($null -eq $line) { $err = $null; break }
                Write-RunLog ($Stage + '/stderr') $line
                $lastOutput = [DateTime]::UtcNow; $received = $true
                $err = $p.StandardError.ReadLineAsync()
            }
            $now = [DateTime]::UtcNow
            if ($ProgressPath -and ($now-$lastProgressPoll).TotalMilliseconds -ge 150) {
                $progressRead=Read-LabProgress $cursor
                foreach ($entry in $progressRead.Lines) { Write-RunLog ($Stage+'/vba') $entry }
                if ($progressRead.Advanced) { $lastOutput=$now; $received=$true }
                $lastProgressPoll=$now
            }
            if ($CancelFile -and [IO.File]::Exists($CancelFile)) { throw 'Run cancelled by caller.' }
            if ($p.HasExited) {
                if ($null -eq $exitSeen) { $exitSeen = $now }
                if ($null -eq $out -and $null -eq $err) { break }
                if (($now - $exitSeen).TotalSeconds -gt 10) { throw 'Child exited but inherited output pipes did not close.' }
            }
            if (($now - $lastHeartbeat).TotalSeconds -ge 5) {
                Write-RunLog $Stage ('Still running: pid={0}, no child output for {1:N1}s' -f $p.Id, ($now - $lastOutput).TotalSeconds)
                $lastHeartbeat = $now
            }
            # Supervisor heartbeats deliberately do NOT reset the stall deadline.
            if ($TimeoutSeconds -gt 0 -and ($now - $lastOutput).TotalSeconds -ge $TimeoutSeconds) {
                throw "$Stage produced no progress for $TimeoutSeconds seconds. See runner.log and the last BEGIN message."
            }
            if (-not $received) { Start-Sleep -Milliseconds 15 }
        }
        if ($ProgressPath) {
            do {
                $progressRead=Read-LabProgress $cursor
                foreach ($entry in $progressRead.Lines) { Write-RunLog ($Stage+'/vba') $entry }
            } while ($progressRead.Advanced)
            $progressRead=Read-LabProgress $cursor -FlushPending
            foreach ($entry in $progressRead.Lines) { Write-RunLog ($Stage+'/vba') $entry }
        }
        $exitCode = $p.ExitCode
        Write-RunLog $Stage "END exit_code=$exitCode"
        if ($exitCode -ne 0) { throw "$Stage failed (exit $exitCode)." }
        return [pscustomobject]@{ exit_code=$exitCode; stdout=($captured.ToArray() -join "`n") }
    } finally {
        if ($started -and -not $p.HasExited) {
            Stop-OwnedExcel
            try { $p.Kill(); [void]$p.WaitForExit(5000) } catch { }
        }
        $p.Dispose()
    }
}
function Invoke-WslText([string[]]$CommandArguments, [string]$Stage) {
    $call = @('-d', $Distro, '--cd', $LinuxRepoRoot, '--exec') + $CommandArguments
    return (Invoke-LoggedProcess -Executable 'wsl.exe' -Arguments $call -Stage $Stage -Capture -TimeoutSeconds 30 -WorkingDirectory $env:TEMP).stdout.Trim()
}
function To-LinuxPath([string]$Path) {
    return Invoke-WslText @('wslpath', '-u', $Path) 'path'
}
function Select-WorkbookInput([string[]]$Names) {
    Add-Type -AssemblyName System.Windows.Forms
    $owner = [Windows.Forms.Form]::new()
    $owner.TopMost=$true; $owner.ShowInTaskbar=$false; $owner.StartPosition='CenterScreen'
    $owner.Width=1; $owner.Height=1; $owner.Opacity=0
    $owner.Show()
    try {
        Write-RunLog 'select' 'Waiting for file/folder selection in the Windows dialog.'
        $choice = [Windows.Forms.MessageBox]::Show($owner, 'Ja: Excel-Datei. Nein: Ordner (ohne Unterordner).', 'Excel JSON diff', 'YesNoCancel')
        if ($choice -eq 'Cancel') { return $null }
        if ($choice -eq 'Yes') {
            $picker=[Windows.Forms.OpenFileDialog]::new()
            $picker.Filter='Excel workbook (*.xlsx)|*.xlsx'
            $picker.CheckFileExists=$true
        } else {
            $picker=[Windows.Forms.FolderBrowserDialog]::new()
            $picker.Description='Ordner mit XLSX-Dateien (ohne Unterordner)'
        }
        try {
            if ($picker.ShowDialog($owner) -ne 'OK') { return $null }
            $selected=if($choice -eq 'Yes'){$picker.FileName}else{$picker.SelectedPath}
        } finally { $picker.Dispose() }
        Write-RunLog 'select' "Selected: $selected; waiting for Shrink profile dialog."
        $form=[Windows.Forms.Form]::new()
        $form.Text='Shrink-Profil'; $form.Width=560; $form.Height=165
        $form.StartPosition='CenterScreen'; $form.TopMost=$true
        $combo=[Windows.Forms.ComboBox]::new()
        $combo.Left=15; $combo.Top=15; $combo.Width=510; $combo.DropDownStyle='DropDownList'
        foreach($name in $Names){[void]$combo.Items.Add($name)}
        $combo.SelectedIndex=[Math]::Max(0,$combo.Items.IndexOf($Profile))
        $button=[Windows.Forms.Button]::new()
        $button.Text='Start';$button.Left=435;$button.Top=60;$button.DialogResult='OK'
        $form.Controls.Add($combo);$form.Controls.Add($button);$form.AcceptButton=$button
        try {
            if($form.ShowDialog($owner) -ne 'OK'){return $null}
            return [pscustomobject]@{ path=$selected; profile=[string]$combo.SelectedItem }
        } finally {$form.Dispose()}
    } finally {$owner.Close();$owner.Dispose()}
}

# Stream and validate TSV runs; never materialize a million row objects in PS.
function Write-RowRuns([IO.TextWriter]$Writer, [string]$Path, [long]$First, [long]$Last) {
    $reader=[IO.StreamReader]::new($Path,[Text.Encoding]::UTF8)
    $next=$First; $pending=$null; $comma=''; $runCount=0
    $progressAt=[DateTime]::UtcNow
    $culture=[Globalization.CultureInfo]::InvariantCulture
    try {
        while($null -ne ($line=$reader.ReadLine())) {
            if($line.Length -eq 0){throw "Empty row record in $Path"}
            $parts=$line.Split([char]9)
            if($parts.Length -ne 4){throw "Malformed row record: $line"}
            $from=[long]::Parse($parts[0],$culture);$to=[long]::Parse($parts[1],$culture)
            $height=[double]::Parse($parts[2],[Globalization.NumberStyles]::Float,$culture)
            if($from -ne $next -or $to -lt $from -or $to -gt $Last){throw "Row coverage gap/overlap/out-of-range in $Path at $from..$to (expected $next)"}
            if([double]::IsNaN($height) -or [double]::IsInfinity($height) -or $height -lt 0){throw "Invalid row height in $Path"}
            if($parts[3] -cne '0' -and $parts[3] -cne '1'){throw "Invalid hidden flag in $Path"}
            $hidden=($parts[3] -ceq '1')
            if($null -ne $pending -and $pending.height -eq $height -and $pending.hidden -eq $hidden) {
                $pending.to=$to
            } else {
                if($null -ne $pending) {
                    $Writer.Write($comma)
                    $Writer.Write("`n          {`"from`":$($pending.from),`"to`":$($pending.to),`"height_points`":$($pending.height.ToString('R',$culture)),`"hidden`":$($pending.hidden.ToString().ToLowerInvariant())}")
                    $comma=','; $runCount++
                }
                $pending=@{from=$from;to=$to;height=$height;hidden=$hidden}
            }
            $next=$to+1
            if(([DateTime]::UtcNow-$progressAt).TotalSeconds -ge 5){
                Write-RunLog 'json' "Validating/canonicalizing rows: through=$to/$Last"
                if($CancelFile -and [IO.File]::Exists($CancelFile)){throw 'Run cancelled by caller.'}
                $progressAt=[DateTime]::UtcNow
            }
        }
        if($next -ne $Last+1 -or $null -eq $pending){throw "Incomplete row collection in ${Path}: stopped before $next, expected through $Last"}
        $Writer.Write($comma)
        $Writer.Write("`n          {`"from`":$($pending.from),`"to`":$($pending.to),`"height_points`":$($pending.height.ToString('R',$culture)),`"hidden`":$($pending.hidden.ToString().ToLowerInvariant())}")
        $Writer.Write("`n")
        return $runCount+1
    } finally {$reader.Dispose()}
}
function Write-ComparisonJson([string]$Variant,[string]$RunRoot,$Manifest) {
    # Windows PowerShell 5.1 emits a top-level JSON array as one pipeline object.
    # Assign it directly; wrapping the pipeline in @() nests that array one level deeper.
    $raw=[IO.File]::ReadAllText((Join-Path $RunRoot "$Variant.raw.json")) | ConvertFrom-Json
    if($raw.Count -ne $Manifest.files.Count){throw "Incomplete $Variant workbook list"}
    $path=Join-Path $RunRoot "$Variant.json.partial"
    $writer=[IO.StreamWriter]::new($path,$false,[Text.UTF8Encoding]::new($false))
    try {
        $writer.WriteLine('[')
        for($f=0;$f -lt $raw.Count;$f++) {
            $book=$raw[$f];$entry=$Manifest.files[$f]
            if($book.id -cne $entry.id -or $book.file -cne $entry.name){throw "Workbook identity mismatch in $Variant"}
            if($f -gt 0){$writer.WriteLine(',')}
            $writer.WriteLine('  {')
            $name=ConvertTo-Json -InputObject ([string]$book.file) -Compress
            $writer.WriteLine('    "file": '+$name+',')
            $writer.WriteLine('    "sheets": [')
            $sheets=@($book.sheets)
            if($sheets.Count -eq 0){throw 'Workbook has no worksheets to compare.'}
            for($s=0;$s -lt $sheets.Count;$s++) {
                $sheet=$sheets[$s]
                if([int]$sheet.index -ne $s+1){throw 'Worksheet index mismatch.'}
                if($s -gt 0){$writer.WriteLine(',')}
                $prefix=(ConvertTo-Json -InputObject $sheet -Depth 12).TrimEnd()
                $prefix=$prefix.Substring(0,$prefix.LastIndexOf('}')).TrimEnd()+','
                foreach($line in ($prefix -split '\r?\n')){$writer.WriteLine('      '+$line)}
                $writer.Write('        "row_heights": [')
                $rows=Join-Path $RunRoot ('rows\'+$entry.id+'\'+$Variant+'-s'+($s+1)+'.tsv')
                $runs=Write-RowRuns $writer $rows ([long]$sheet.row_scan.first_row) ([long]$sheet.row_scan.last_row)
                if([long]$sheet.row_scan.measured_rows -ne [long]$sheet.row_scan.last_row-[long]$sheet.row_scan.first_row+1){throw 'Invalid measured row count.'}
                $writer.WriteLine('        ]')
                $writer.Write('      }')
                Write-RunLog 'json' "$Variant $($book.file) sheet=$($sheet.name): rows=$($sheet.row_scan.measured_rows), exact height runs=$runs"
            }
            $writer.WriteLine();$writer.WriteLine('    ]');$writer.Write('  }')
        }
        $writer.WriteLine();$writer.WriteLine(']')
    } finally {$writer.Dispose()}
    return $path
}
function Open-WindowsDiff([string]$Original,[string]$Shrunk) {
    $candidates=[Collections.Generic.List[string]]::new()
    $cmd=Get-Command code.cmd -ErrorAction SilentlyContinue
    if($cmd){$candidates.Add((Join-Path (Split-Path (Split-Path $cmd.Source -Parent) -Parent) 'Code.exe'))}
    if($env:LOCALAPPDATA){$candidates.Add((Join-Path $env:LOCALAPPDATA 'Programs\Microsoft VS Code\Code.exe'))}
    if($env:ProgramFiles){$candidates.Add((Join-Path $env:ProgramFiles 'Microsoft VS Code\Code.exe'))}
    foreach($exe in $candidates) {
        $js=Join-Path (Split-Path $exe -Parent) 'resources\app\out\cli.js'
        if([IO.File]::Exists($exe) -and [IO.File]::Exists($js)) {
            # Avoid cmd.exe/.cmd argument re-interpretation (%, &, quotes, etc.).
            [void](Invoke-LoggedProcess -Executable $exe -Arguments @($js,'--reuse-window','--diff',$Original,$Shrunk) -Stage 'diff' -TimeoutSeconds 60 -Environment @{ELECTRON_RUN_AS_NODE='1'})
            return
        }
    }
    throw "VS Code CLI not found. The JSON files are retained: $Original ; $Shrunk"
}

if($FunctionsOnly){return}
$exitCode=0
try {
    if([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT){throw 'Use Windows powershell.exe; Linux pwsh cannot automate desktop Excel.'}
    if([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA'){throw 'Start with powershell.exe -STA.'}
    $OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
    $repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
    $run=Join-Path $repo ('.tmp\excel-json-diff\'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
    [void][IO.Directory]::CreateDirectory($run)
    $script:RunLog=[IO.StreamWriter]::new((Join-Path $run 'runner.log'),$false,[Text.UTF8Encoding]::new($false))
    $script:RunLog.AutoFlush=$true
    $script:OwnerToken=[guid]::NewGuid().ToString('N')
    $script:ExcelState=Join-Path $run 'excel-process.json'
    Write-RunLog 'start' "Results/log: $run"
    Write-RunLog 'start' "Runtime: PowerShell supervisor -> Rust shrink -> PowerShell/Excel COM -> VBA; row_scope=$RowScope, batch_rows=$BatchRows"
    if($ResultFile -and [IO.File]::Exists($ResultFile)){throw 'ResultFile already exists; use a new file.'}
    if($CancelFile -and [IO.File]::Exists($CancelFile)){throw 'Cancellation file already exists; use a new launch.'}
    # Windows VS Code opened directly on the WSL UNC checkout.
    if(-not $LinuxRepoRoot -and $repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.+)$') {
        $Distro=$Matches[1];$LinuxRepoRoot='/'+$Matches[2].Replace('\','/')
    }
    if($LinuxRepoRoot -and -not $Distro){throw 'A WSL distro is required when LinuxRepoRoot is set.'}
    $profileData=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'json_diff_profiles.json')) | ConvertFrom-Json
    $profileSource=[IO.File]::ReadAllText((Join-Path $repo 'excel-shrink-cli\src\visual_profile.rs')).Replace("`r`n","`n")
    $definition=[regex]::Match($profileSource,'(?s)pub const PREDEFINED_VISUAL_PROFILES[^=]*=\s*\[(.*?)\];')
    if(-not $definition.Success -or (Get-TextSha256 $definition.Groups[1].Value) -cne $profileData.definition_sha256){throw 'Rust profile definitions changed. Rerun patch_excel_json_diff.py to refresh the profile snapshot.'}
    $profiles=@($profileData.profiles)
    if(-not $InputPath) {
        $selection=Select-WorkbookInput @($profiles | ForEach-Object {$_.name})
        if($null -eq $selection){Write-RunLog 'select' 'Cancelled. No shrink or Excel measurement was started.';return}
        $InputPath=$selection.path;$Profile=$selection.profile
    }
    $selectedProfile=@($profiles | Where-Object {$_.name -ceq $Profile})
    if($selectedProfile.Count -ne 1){throw "Unknown profile: $Profile"}
    if($LinuxRepoRoot -and $InputPath.StartsWith('/')){ $InputPath=Invoke-WslText @('wslpath','-w',$InputPath) 'path' }
    if($LinuxRepoRoot -and $HelperWorkbook -and $HelperWorkbook.StartsWith('/')){$HelperWorkbook=Invoke-WslText @('wslpath','-w',$HelperWorkbook) 'path'}
    $source=(Get-Item -LiteralPath $InputPath).FullName
    if([IO.File]::Exists($source)){$files=@(Get-Item -LiteralPath $source)}else{$files=@(Get-ChildItem -LiteralPath $source -File)}
    $files=@($files | Where-Object {$_.Extension -ieq '.xlsx' -and -not $_.Name.StartsWith('~$')} | Sort-Object FullName)
    if($files.Count -eq 0){throw 'No XLSX files selected. Folder selection is non-recursive; lock files are skipped.'}
    Write-RunLog 'select' "Files=$($files.Count); profile=$Profile; source=$source"
    if($RowScope -eq 'all'){Write-RunLog 'measure-plan' 'All worksheet rows will be measured, including rows outside UsedRange. This is an exhaustive scan, not sampling.'}
    else{Write-RunLog 'measure-plan' 'LIMITED scope: row 1 through the larger original/shrunk UsedRange last row. The scope is recorded in both JSONs.'}
    $bas=Join-Path $PSScriptRoot 'JsonDiffProbe.bas'
    $basText=[IO.File]::ReadAllText($bas)
    $version=[regex]::Match($basText,'JsonDiffVersion\s*=\s*"([^"]+)"').Groups[1].Value
    if(-not $version){throw 'Missing VBA probe version.'}
    $sourceHash=(Get-FileHash -LiteralPath $bas -Algorithm SHA256).Hash.ToLowerInvariant()
    $cache=Join-Path $repo ('.tmp\excel-json-diff-helper\'+$sourceHash+'\ProbeHost.xlsm')
    if(-not $HelperWorkbook){
        $manual=Join-Path $PSScriptRoot 'JsonDiffProbeHost.xlsm'
        if([IO.File]::Exists($manual)){$HelperWorkbook=$manual}
    }
    if($HelperWorkbook){$HelperWorkbook=(Get-Item -LiteralPath $HelperWorkbook).FullName}
    $request=[ordered]@{action='prepare';run_root=$run;owner_token=$script:OwnerToken;state_file=$script:ExcelState;
        bas_path=$bas;probe_version=$version;cache_path=$cache;helper_workbook=$HelperWorkbook;batch_rows=$BatchRows;row_scope=$RowScope}
    $requestPath=Join-Path $run 'request.json'
    Write-JsonFile $request $requestPath
    $shell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $worker=Join-Path $PSScriptRoot 'json_diff_excel.ps1'
    # Preflight before spending time shrinking: Excel, VBA access/host, probe version, self-test.
    try {[void](Invoke-LoggedProcess -Executable $shell -Arguments @('-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',$worker,'-RequestFile',$requestPath) -Stage 'excel-preflight' -TimeoutSeconds $StallSeconds -WorkingDirectory $env:TEMP)}
    finally {Stop-OwnedExcel}
    $inputDir=Join-Path $run 'input';$shrunkDir=Join-Path $run 'shrunk'
    [void][IO.Directory]::CreateDirectory($inputDir);[void][IO.Directory]::CreateDirectory($shrunkDir)
    $entries=[Collections.Generic.List[object]]::new()
    for($i=0;$i -lt $files.Count;$i++){
        $id=$i.ToString('D6');$file=$files[$i];$copy=Join-Path $inputDir ($id+'.xlsx')
        Write-RunLog 'stage' "Copy/hash $($i+1)/$($files.Count): $($file.FullName)"
        $before=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        [IO.File]::Copy($file.FullName,$copy,$false)
        $after=(Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash
        if($before -cne $after){throw "Input changed during copy: $($file.FullName)"}
        $entries.Add([ordered]@{id=$id;name=$file.Name;source=$file.FullName;sha256=$after.ToLowerInvariant()})
    }
    $manifest=[ordered]@{schema='excel-json-diff-v1';profile=$Profile;flags=@($selectedProfile[0].flags);row_scope=$RowScope;batch_rows=$BatchRows;files=$entries.ToArray();probe=([IO.File]::ReadAllText((Join-Path $run 'probe-info.json')) | ConvertFrom-Json)}
    Write-JsonFile $manifest (Join-Path $run 'manifest.json')
    $common=@('run','--release','--locked','-p','excel-shrink-cli','--bin','excel-shrink-cli','--')
    $shrinkWatch=[Diagnostics.Stopwatch]::StartNew()
    if($LinuxRepoRoot){
        $wslHome=Invoke-WslText @('printenv','HOME') 'wsl'
        $linuxInput=To-LinuxPath $inputDir;$linuxOutput=To-LinuxPath $shrunkDir
        $native=@('-d',$Distro,'--cd',$LinuxRepoRoot,'--exec',($wslHome+'/.cargo/bin/cargo'))+$common+@($linuxInput,$linuxOutput)+@($selectedProfile[0].flags)+@('--console-log-level','info')
        [void](Invoke-LoggedProcess -Executable 'wsl.exe' -Arguments $native -Stage 'shrink' -TimeoutSeconds $StallSeconds -WorkingDirectory $env:TEMP)
    }else{
        $cargo=Get-Command cargo.exe -ErrorAction SilentlyContinue
        $cargoPath=if($cargo){$cargo.Source}else{Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'}
        if(-not [IO.File]::Exists($cargoPath)){throw 'Native Windows cargo.exe not found. Open the WSL checkout or install a Windows Rust toolchain.'}
        [void](Invoke-LoggedProcess -Executable $cargoPath -Arguments ($common+@($inputDir,$shrunkDir)+@($selectedProfile[0].flags)+@('--console-log-level','info')) -Stage 'shrink' -TimeoutSeconds $StallSeconds -WorkingDirectory $repo)
    }
    $manifest['build_and_shrink_seconds']=$shrinkWatch.Elapsed.TotalSeconds
    foreach($entry in $entries){
        $output=Join-Path $shrunkDir ($entry.id+'.xlsx')
        if(-not [IO.File]::Exists($output) -or ([IO.FileInfo]$output).Length -eq 0){throw "Rust returned without the expected non-empty output: $output"}
        $entry['shrunk_sha256']=(Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-RunLog 'shrink' "OUTPUT $output"
    }
    Write-JsonFile $manifest (Join-Path $run 'manifest.json')
    $request['action']='measure';Write-JsonFile $request $requestPath
    try {[void](Invoke-LoggedProcess -Executable $shell -Arguments @('-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',$worker,'-RequestFile',$requestPath) -Stage 'excel' -TimeoutSeconds $StallSeconds -WorkingDirectory $env:TEMP)}
    finally {Stop-OwnedExcel}
    $complete=[IO.File]::ReadAllText((Join-Path $run 'measurement-complete.json')) | ConvertFrom-Json
    if(-not $complete.complete -or $complete.files -ne $entries.Count){throw 'Excel collection is incomplete.'}
    $left=Write-ComparisonJson 'original' $run $manifest
    $right=Write-ComparisonJson 'shrunk' $run $manifest
    $original=Join-Path $run 'original.json';$shrunk=Join-Path $run 'shrunk.json'
    [IO.File]::Move($left,$original);[IO.File]::Move($right,$shrunk)
    $identical=(Get-FileHash -LiteralPath $original).Hash -ceq (Get-FileHash -LiteralPath $shrunk).Hash
    Write-RunLog 'done' "JSON identical=$identical (UsedRange differences are included; differences alone are not a layout-failure verdict)."
    Write-RunLog 'done' "ORIGINAL JSON: $original"
    Write-RunLog 'done' "SHRUNK JSON:   $shrunk"
    if(-not $NoDiff){
        if($DeferDiff){
            if(-not $ResultFile){throw 'DeferDiff requires ResultFile.'}
            [IO.File]::WriteAllText($ResultFile,($original+"`n"+$shrunk+"`n"),[Text.UTF8Encoding]::new($false))
            Write-RunLog 'diff' 'Returning diff paths to the original WSL/VS Code session.'
        }else{Open-WindowsDiff $original $shrunk}
    }
}catch{
    $exitCode=1
    Write-RunLog 'ERROR' $_.Exception.Message
    if($_.ScriptStackTrace){Write-RunLog 'ERROR' $_.ScriptStackTrace}
}finally{
    Stop-OwnedExcel
    if($null -ne $script:RunLog){$script:RunLog.Dispose();$script:RunLog=$null}
}
exit $exitCode
