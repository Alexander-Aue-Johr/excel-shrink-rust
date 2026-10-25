#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
powershell_path=$(wslpath -u 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe')
script_path=$(wslpath -w "$repo_root/tools/start-corpus-wsl.ps1")

exec "$powershell_path" -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File "$script_path" \
    -LinuxRepo "$repo_root" -Distribution "${WSL_DISTRO_NAME:?Run this launcher inside WSL}" "$@"
