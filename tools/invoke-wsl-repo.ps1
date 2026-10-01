param(
    [Parameter(Mandatory=$true)]
    [string]$RepoRoot,

    [Parameter(Mandatory=$true)]
    [string]$Executable,

    [Parameter(Mandatory=$true)]
    [string]$ArgumentsJson
)

$ErrorActionPreference = 'Stop'

function Resolve-WslRepo([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)

    if ($full -match '^\\\\wsl(?:\.localhost|\$)?\\([^\\]+)\\(.*)$') {
        return [pscustomobject]@{
            Distribution = $Matches[1]
            LinuxPath = '/' + ($Matches[2] -replace '\\', '/')
            WindowsRoot = $full.TrimEnd([char]'\')
        }
    }

    $linuxPath = (
        & wsl.exe -- wslpath -a -u $full
    ).Trim()

    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($linuxPath)) {
        throw "Could not map repository path into WSL: $full"
    }

    return [pscustomobject]@{
        Distribution = $null
        LinuxPath = $linuxPath.TrimEnd([char]'/')
        WindowsRoot = $full.TrimEnd([char]'\')
    }
}

$repo = Resolve-WslRepo $RepoRoot

try {
    $ExecutableArgs = @($ArgumentsJson | ConvertFrom-Json)
}
catch {
    throw "ArgumentsJson is not valid JSON: $($_.Exception.Message)"
}

$translatedArgs = @(
    foreach ($arg in $ExecutableArgs) {
        $text = [string]$arg

        if (
            $text.StartsWith(
                $repo.WindowsRoot,
                [StringComparison]::OrdinalIgnoreCase
            )
        ) {
            $suffix = $text.Substring($repo.WindowsRoot.Length)
            $suffix = $suffix -replace '\\', '/'
            $suffix = $suffix.TrimStart([char]'/')

            if ([string]::IsNullOrWhiteSpace($suffix)) {
                $repo.LinuxPath
            }
            else {
                $repo.LinuxPath.TrimEnd([char]'/') + '/' + $suffix
            }
        }
        else {
            $text
        }
    }
)

$wslArgs = @()

if (-not [string]::IsNullOrWhiteSpace($repo.Distribution)) {
    $wslArgs += @('-d', $repo.Distribution)
}

$wslArgs += @(
    '--cd',
    $repo.LinuxPath,
    '--',
    'bash',
    '-lc',
    'exec "$@"',
    'bash',
    $Executable
)
$wslArgs += $translatedArgs

& wsl.exe @wslArgs
exit $LASTEXITCODE
