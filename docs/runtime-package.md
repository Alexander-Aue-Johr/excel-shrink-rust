# Windows runtime package

Build the Windows entry points with a configured build directory on a working
drive, then package their outputs into a new directory. Python 3.9 or newer and
Git are required by the packaging tool. The resulting runtime requires Windows
and installed Excel for native comparisons.

```powershell
cargo build -p excel-shrink-cli --bins --release -j 2
python tools/package-validation.py `
  --binary-directory "$env:CARGO_TARGET_DIR\release" `
  --output 'V:\releases\deltasieve-candidate'
python tools/package-validation.py --verify 'V:\releases\deltasieve-candidate'
```

The parent output directory must already exist. The tool rejects an existing
destination. An interrupted copy retains an `.incomplete` marker and cannot pass
verification. Reattempt publication into a new directory after inspection.

The package contains supplied `deltasieve.exe` and `excel-shrink-cli.exe`, tracked
runtime Office/web assets and their PowerShell/C# process-management helpers.
It excludes test directories, local corpora, generated dependencies and ordinary
source files. `release-manifest.json` records every packaged member's byte size
and SHA-256. Verification checks required members, hashes, unexpected files and
path confinement after relocation.

The asset checkout commit and dirty state are recorded separately from supplied
binary hashes. The packager does not infer which source revision produced an
executable. Preserve the actual build receipt when freezing a scientific
release. A verified package proves byte integrity against its local manifest,
with no claim of independent signature-based authenticity.

Run `deltasieve.exe` to open the standalone configurator. Keep the complete
directory layout when moving it. Use a working drive for TEMP, TMP,
EXCEL_SHRINK_LOG_DIR and EXCEL_SHRINK_VISUAL_LOG_DIR. Supplied configurations
should put corpus inputs, prepared states and outputs on working drives too.
The Office asset resolver supports an explicit EXCEL_SHRINK_CAPTURE_DIR for
controlled deployments. An invalid explicit path fails.

This is a packaging prerequisite, not the final distributed evaluation release.
The full roadmap still requires independent application defaults, replay,
export/merge and ordinary study start/resume acceptance after relocation.
