#!/usr/bin/env bash
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
command -v powershell.exe >/dev/null || { echo 'Windows powershell.exe required' >&2; exit 1; }
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File "$(wslpath -w "$here/pure.ps1")" -ToolRoot "$(wslpath -w "$here/..")"
