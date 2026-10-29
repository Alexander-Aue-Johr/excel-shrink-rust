[CmdletBinding()]
param([switch]$WrapperTest)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1') -Force
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
foreach ($name in @('start-corpus-terminal.ps1', 'install-corpus-command.ps1')) {
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $repo "tools/$name"), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "Syntax errors in $name" }
    foreach ($function in $ast.FindAll({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst]
    }, $false)) { . ([scriptblock]::Create($function.Extent.Text)) }
}
$script:checks = 0
function Assert($condition, [string]$message) { if (-not $condition) { throw $message }; $script:checks++ }
function Reject([scriptblock]$action, [string]$message) {
    $rejected = $false
    try { & $action | Out-Null } catch { $rejected = $true }
    Assert $rejected $message
}

# The production entry functions call these dependencies unchanged. No tests
# launch the corpus runner, build tools, Excel, a server, or the user's browser.
function Import-StudyConfiguration {
    param($Repo, $ConfigurationPath)
    # Importing a saved configuration returns a fresh object for each command.
    return ($script:configuration | ConvertTo-Json -Depth 10 | ConvertFrom-Json)
}
function Assert-StudySettings { param($Settings) }
function Get-StudyRunStatus { param($OutputRoot); return $script:status }
function Read-StudyJson { param($Path); return $script:receipt }
function New-StudyLaunchPlan {
    param($Configuration, $Repo, $Executable, $RunDirectory)
    $script:planned = @{
        corpus=$Configuration.settings.corpus; partition=$Configuration.settings.partition
        partitions=$Configuration.settings.partitions; order=$Configuration.settings.order
        build=$Configuration.settings.build_release; directory=$RunDirectory
        aggressive_first=(Get-Value $Configuration.settings 'aggressive_first' $false)
    }
    return @{ Executable=$Executable }
}
function Start-StudyLaunch { param($Configuration, $ConfigurationPath, $Repo, $Plan); $script:launches++; return $script:receipt }
function Invoke-RestMethod { param($Uri, $TimeoutSec, $ErrorAction); return $script:health }
function Open-CorpusObservatory { param($Url); $script:browserOpens++ }

$root = Join-Path $repo ('.tmp/terminal-command-tests/' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$beforeLocation = Get-Location
try {
    $configPath = Join-Path $root 'config.json'
    [IO.File]::WriteAllText($configPath, '{}')
    $script:configuration = [pscustomobject]@{
        corpora=@([pscustomobject]@{name='Enron'}, [pscustomobject]@{name='Destatis'})
        settings=[pscustomobject]@{
            output_root='.tmp/saved output'; control_room_port=8771; corpus='Destatis'
            partition=1; partitions=4; order='size-desc'; build_release=$false
        }
    }
    $selection = @{Corpus='Enron'; Partition='3/4'}
    $output = Resolve-StudyPath $root $configuration.settings.output_root
    $script:receipt = [pscustomobject]@{
        Pid=42424; ControlRoom='http://127.0.0.1:8771/'; Stdout='fake stdout.log'; Stderr='fake stderr.log'
        Settings=[pscustomobject]@{corpus='Enron';partition=3;partitions=4;order='size-desc'}
    }
    $script:health = [pscustomobject]@{ kind='excel-shrink-control-room'; root=(Join-Path $output 'control-room') }
    $script:status = [pscustomobject]@{ Busy=$false; Process=$null }
    $script:launches = 0; $script:browserOpens = 0
    $elsewhere = Join-Path $root 'unrelated working directory'
    [void][IO.Directory]::CreateDirectory($elsewhere)
    Set-Location -LiteralPath $elsewhere
    $result = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection
    Assert ($launches -eq 1 -and $browserOpens -eq 1 -and -not $result.AlreadyRunning) 'Saved run did not start and open its dashboard.'
    Assert ($planned.partition -eq 3 -and $planned.directory.StartsWith($root)) 'The current working directory changed the saved partition or run location.'
    Assert ($planned.corpus -eq 'Enron' -and $planned.partitions -eq 4 -and $planned.order -eq 'size-desc') 'Explicit corpus and quarter were not used with the saved processing order.'
    Assert ($configuration.settings.corpus -eq 'Destatis' -and $configuration.settings.partition -eq 1) 'A command selection changed the saved configuration.'
    Assert (-not $planned.build) 'A saved SkipBuild configuration was ignored.'
    $script:status = [pscustomobject]@{ Busy=$true; Process=[pscustomobject]@{Id=42424} }
    $result = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection
    Assert ($launches -eq 1 -and $browserOpens -eq 2 -and $result.AlreadyRunning) 'An active run was started twice instead of opening its dashboard.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert ($launches -eq 1 -and $browserOpens -eq 2) '-NoBrowser launched a browser or another runner.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Destatis -Partition 3/4 } 'A request for another corpus opened an unrelated active run.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron -Partition 2/4 } 'A request for another quarter opened an unrelated active run.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron -Partition all } 'A whole-corpus request opened an active quarter run.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Order size-asc } 'A request for another processing order opened an unrelated active run.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Order priority } 'A priority request opened an active size-desc run.'
    Assert ($launches -eq 1 -and $browserOpens -eq 2) 'A selection mismatch launched work or opened the wrong dashboard.'
    $health.root = Join-Path $root 'another output/control-room'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection } 'A foreign output root was accepted as this run.'
    Assert ($browserOpens -eq 2 -and $launches -eq 1) 'A foreign dashboard was opened or a second run was started.'
    $health.root = Join-Path $output 'control-room'; $health.kind = 'unrelated-service'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection } 'An unrelated HTTP service was accepted.'
    $health.kind = 'excel-shrink-control-room'
    $script:status = [pscustomobject]@{ Busy=$true; Process=$null }
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection } 'An unverified busy output started a second runner.'
    $script:status = [pscustomobject]@{ Busy=$false; Process=$null }
    $configuration.settings.build_release = $true
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert $planned.build 'A saved build preference was ignored.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -SkipBuild -NoBrowser
    Assert (-not $planned.build) '-SkipBuild did not override the saved build preference.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Build -NoBrowser
    Assert $planned.build '-Build did not request a release build.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Build -SkipBuild } 'Contradictory build options were accepted.'
    Reject { Start-SavedCorpusRun $root (Join-Path $root 'missing.json') 'fake runner.exe' @selection } 'A missing saved configuration silently started a default run.'

    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Destatis -Partition 2/4 -NoBrowser
    Assert ($planned.corpus -eq 'Destatis' -and $planned.partition -eq 2 -and $planned.partitions -eq 4) 'Another corpus and quarter were not forwarded to the launch plan.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron -Partition all -NoBrowser
    Assert ($planned.corpus -eq 'Enron' -and $planned.partition -eq 0) 'The whole-corpus selection was not forwarded to the launch plan.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Order size-asc -NoBrowser
    Assert ($planned.order -eq 'size-asc') 'An explicit processing order was ignored.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert ($planned.order -eq 'size-desc' -and $configuration.settings.order -eq 'size-desc') 'A previous order override leaked into the next command or the saved settings.'
    $configuration.settings.order = 'size-asc'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert ($planned.order -eq 'size-asc') 'Omitting the processing order did not retain a saved non-default order.'
    $configuration.settings.order = 'size-desc'

    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Order priority -NoBrowser
    Assert ($planned.order -eq 'priority' -and $configuration.settings.order -eq 'size-desc') 'An explicit priority order was rejected, lost, or leaked into saved settings.'
    $configuration.settings.order = 'priority'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert ($planned.order -eq 'priority') 'Omitting Order lost the saved priority order.'
    $configuration.settings.order = 'size-desc'

    # The switch changes one invocation; saved strategy and the strategy of an
    # already active run are separate sources of truth that must not be merged.
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -AggressiveFirst -NoBrowser
    Assert $planned.aggressive_first '-AggressiveFirst was not forwarded to the plan.'
    Assert (-not (Get-Value $configuration.settings 'aggressive_first' $false)) 'The strategy override changed the source configuration.'
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert (-not $planned.aggressive_first) 'An aggressive-first override leaked into the next regular invocation.'
    $configuration.settings | Add-Member -NotePropertyName aggressive_first -NotePropertyValue $true -Force
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert $planned.aggressive_first 'Omitting the strategy switch lost saved aggressive-first selection.'

    $script:status = [pscustomobject]@{ Busy=$true; Process=[pscustomobject]@{Id=42424} }
    $receipt.Settings | Add-Member -NotePropertyName aggressive_first -NotePropertyValue $true -Force
    $beforeStrategyLaunches = $launches
    $beforeStrategyBrowsers = $browserOpens
    $result = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser
    Assert ($result.AlreadyRunning -and $launches -eq $beforeStrategyLaunches) 'A matching saved aggressive-first strategy started a second runner.'
    $configuration.settings.aggressive_first = $false
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser } 'A normal request accepted an active aggressive-first run.'
    $result = Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -AggressiveFirst -NoBrowser
    Assert ($result.AlreadyRunning -and $launches -eq $beforeStrategyLaunches) 'Explicit aggressive-first did not match an already active screening run.'

    $receipt.Settings.aggressive_first = $false
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -AggressiveFirst -NoBrowser } 'Aggressive-first opened an active full-validation run.'
    $configuration.settings.aggressive_first = $true
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -NoBrowser } 'Saved aggressive-first opened an active full-validation run.'
    Assert ($launches -eq $beforeStrategyLaunches -and $browserOpens -eq $beforeStrategyBrowsers) 'A strategy mismatch started work or opened an unrelated dashboard.'
    $configuration.settings.aggressive_first = $false
    $receipt.Settings.PSObject.Properties.Remove('aggressive_first')
    $script:status = [pscustomobject]@{ Busy=$false; Process=$null }
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Destatis -Standalone -NoBrowser } 'A normal configuration was accepted as a standalone deployment.'
    $configuration | Add-Member -NotePropertyName standalone_windows -NotePropertyValue $true
    $configuration | Add-Member -NotePropertyName standalone_corpora -NotePropertyValue @('Destatis')
    $configuration.settings.build_release = $true
    $null = Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Destatis -Standalone -NoBrowser
    Assert (-not $planned.build -and $planned.corpus -eq 'Destatis' -and $planned.partition -eq 1) 'Standalone launch rebuilt or lost its saved selection.'
    Assert $configuration.settings.build_release 'Standalone launch changed the normal build preference.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Destatis -Standalone -Build -NoBrowser } 'Standalone launch accepted a Windows rebuild.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron -Partition all -Standalone -NoBrowser } 'An unprepared standalone corpus was accepted.'
    $configuration.standalone_windows = $false
    $configuration.settings.build_release = $false

    $beforeInvalidLaunches = $launches; $beforeInvalidBrowsers = $browserOpens
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Partition 3/4 } 'A missing corpus silently selected saved work.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron } 'A missing partition silently selected saved work.'
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Unknown -Partition 3/4 } 'An unknown corpus silently selected saved work.'
    foreach ($invalidPartition in @('0/4','5/4','1/0','1/1001','1001/1001','1.0/4','3/4/5','-1/4','99999999999999999999999/4','1/99999999999999999999999')) {
        Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' -Corpus Enron -Partition $invalidPartition } "Invalid partition was accepted: $invalidPartition"
    }
    Reject { Start-SavedCorpusRun $root $configPath 'fake runner.exe' @selection -Order ascending } 'An invalid processing order was accepted.'
    Assert ($launches -eq $beforeInvalidLaunches -and $browserOpens -eq $beforeInvalidBrowsers) 'An invalid selection launched work or opened a dashboard.'

    $bin = Join-Path $root 'command bin'
    Assert ((Add-CorpusCommandPath 'C:\existing' $bin) -eq ('C:\existing;' + $bin)) 'The command directory was not appended to PATH.'
    Assert ((Add-CorpusCommandPath ('C:\existing;' + $bin.ToUpperInvariant() + '\;') $bin) -eq ('C:\existing;' + $bin.ToUpperInvariant() + '\;')) 'An existing PATH entry was duplicated or rewritten.'
    Assert ((Add-CorpusCommandPath '' $bin) -eq $bin) 'An empty user PATH was not handled.'
    $entryDirectory = Join-Path $root ('repo with spaces & ! % caf' + [char]0x00E9)
    [void][IO.Directory]::CreateDirectory($entryDirectory)
    $entry = Join-Path $entryDirectory 'entry.ps1'
    @'
param([string]$Output, [string]$Value, [string]$Corpus, [string]$Partition, [string]$Order, [switch]$NoBrowser, [switch]$AggressiveFirst)
[IO.File]::WriteAllText($Output, (@{value=$Value;corpus=$Corpus;partition=$Partition;order=$Order;no_browser=[bool]$NoBrowser;aggressive_first=[bool]$AggressiveFirst} | ConvertTo-Json))
exit 7
'@ | Set-Content -LiteralPath $entry -Encoding UTF8
    $command = Install-CorpusCommand $bin $entry -NoPathUpdate
    Assert ([IO.Path]::GetFileName($command) -ceq 'excel-shrink-visual-validation.cmd') 'The installed command name does not describe visual validation.'
    $contents = [IO.File]::ReadAllText($command)
    $written = [IO.File]::GetLastWriteTimeUtc($command)
    $null = Install-CorpusCommand $bin $entry -NoPathUpdate
    Assert ([IO.File]::ReadAllText($command) -ceq $contents -and [IO.File]::GetLastWriteTimeUtc($command) -eq $written) 'Reinstalling the same command rewrote it.'
    $foreign = Join-Path $root 'unrelated bin'
    [void][IO.Directory]::CreateDirectory($foreign)
    $foreignCommand = Join-Path $foreign 'excel-shrink-visual-validation.cmd'
    [IO.File]::WriteAllText($foreignCommand, '@echo keep this command')
    Reject { Install-CorpusCommand $foreign $entry -NoPathUpdate } 'An unknown command was overwritten.'
    Assert ([IO.File]::ReadAllText($foreignCommand) -ceq '@echo keep this command') 'The rejected install changed another command.'
    [IO.File]::WriteAllText($foreignCommand, '')
    Reject { Install-CorpusCommand $foreign $entry -NoPathUpdate } 'An unknown empty command was overwritten.'
    Assert (([IO.File]::ReadAllBytes($foreignCommand)).Length -eq 0) 'The rejected install changed the empty command.'

    if ($WrapperTest) {
        $capture = Join-Path $root 'arguments.json'
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $env:ComSpec
        $info.Arguments = '/d /c ""' + $command + '" -Output "' + $capture + '" -Value "hello & world !" -Corpus Enron -Partition 3/4 -Order priority -NoBrowser -AggressiveFirst"'
        $info.WorkingDirectory = $elsewhere
        $info.UseShellExecute = $false; $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        $process = [Diagnostics.Process]::new(); $process.StartInfo = $info
        try {
            [void]$process.Start()
            $stdout = $process.StandardOutput.ReadToEndAsync(); $stderr = $process.StandardError.ReadToEndAsync()
            if (-not $process.WaitForExit(20000)) { $process.Kill(); throw 'The synthetic command wrapper timed out.' }
            Assert ($process.ExitCode -eq 7) ('The wrapper lost the entry script exit code. ' + $stderr.Result)
            $actual = [IO.File]::ReadAllText($capture) | ConvertFrom-Json
            Assert ($actual.value -ceq 'hello & world !' -and $actual.no_browser) 'The wrapper changed quoted arguments or switches.'
            Assert ($actual.corpus -ceq 'Enron' -and $actual.partition -ceq '3/4' -and $actual.order -ceq 'priority' -and $actual.aggressive_first) 'The wrapper changed corpus, partition, priority order, or strategy arguments.'
        } finally { $process.Dispose() }
    }
    Write-Host "PASS: $checks terminal command checks; saved settings, strategy guards, priority order, active-run ownership, browser choice, PATH, wrapper preservation."
} finally {
    Set-Location -LiteralPath $beforeLocation
    $prefix = [IO.Path]::GetFullPath((Join-Path $repo '.tmp/terminal-command-tests')).TrimEnd('\') + '\'
    if (-not [IO.Path]::GetFullPath($root).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($root) -notmatch '^[a-f0-9]{32}$') { throw 'Unexpected terminal-test directory.' }
    Remove-Item -LiteralPath $root -Recurse -Force
}
