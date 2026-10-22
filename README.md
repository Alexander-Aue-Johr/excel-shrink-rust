# excel-shrink-rust

`excel-shrink-rust` is a Rust implementation of Excel Shrink. It processes `.xlsx` files and rewrites them in a reduced form. The current implementation focuses on cleaning XLSX-internal XML data and reducing unnecessary content within Excel workbooks.

The repository is organised as a Rust Cargo workspace with two crates:

* `excel-shrink-core`: library code for cleaning XLSX-internal XML data
* `excel-shrink-cli`: command-line program for running Excel Shrink on one file or a directory of files

## Repository structure

```text
excel-shrink-rust/
├── Cargo.toml
├── Cargo.lock
├── excel-shrink-core/
│   ├── Cargo.toml
│   └── src/
│       ├── error.rs
│       ├── lib.rs
│       ├── models.rs
│       ├── sheet.rs
│       ├── styles.rs
│       ├── tests.rs
│       └── workbook.rs
└── excel-shrink-cli/
    ├── Cargo.toml
    └── src/
        ├── main.rs
        └── pipeline.rs
```

## What the tool does

The CLI accepts either a single `.xlsx` file or a directory containing `.xlsx` files. It opens each XLSX file as a ZIP archive, processes the relevant XML files, and writes a cleaned XLSX file to the output directory.

The cleanup parser currently requires unprefixed SpreadsheetML elements. If any XML part
declares a prefix for that namespace, the CLI copies the **entire original XLSX byte for byte**
and logs `workbook_preserved_unsupported_namespace` and `file_preserved`. No cleanup is
performed and the file-size reduction is zero, including with stronger cleanup flags.
Strict SpreadsheetML, UTF-16 XML, and namespace declarations the preflight cannot safely
check receive the same fallback. A passing visual comparison of these unchanged files
demonstrates preservation; it does not demonstrate successful shrinking.

Study reports can diagnose visual differences by rerunning the affected profile with
only whitespace-cell deletion disabled. New comparisons run this control when needed;
resuming also fills in missing diagnoses for completed files with differences. Use
`regression study --diagnose-whitespace-only` with the usual corpus and output arguments
to perform only that backfill. Primary results and their evidence are retained.

A whitespace attribution requires a complete unchanged control on the affected sheet.
For older results, the source, baseline and profile fingerprints must match, and the
stored before/after image pixels must be reproduced. Missing or failed diagnostics stay
unresolved. Height changes remain explicitly marked as not fully pixel-checked.
Reports put unresolved files first, retain whole workbook entries, and provide category
switches that collapse evidence without changing results or counts. On mixed sheets,
the control's remaining differences appear before the original comparisons.

The main command-line options are:

```text
excel-shrink-cli <INPUT_PATH> <OUTPUT_PATH> [OPTIONS]

Arguments:
  <INPUT_PATH>   Path to an .xlsx file or to a directory containing .xlsx files
  <OUTPUT_PATH>  Path to the output directory

Options:
  --recursive                 Recursively search input directories for .xlsx files
  --disable-multiprocessing   Run in single-threaded mode
  --chunk-size <BYTES>        Set the XML processing chunk size
  --force-overwrite           Overwrite existing output files
  --working-directory <PATH>  Resolve relative paths from this directory
```

The default chunk size is `16777216` bytes, which is 16 MiB.
The separate `--parallel-range-chunk-size` option also defaults to 16 MiB of uncompressed
XML per row-aligned work item. Use `0` to disable range chunking.

### Visual-preservation cleanup flags

The default run is conservative for visible workbook layout: textless visual representatives and
unreferenced whitespace-only cells are kept when removing them could slightly change the rendered
worksheet. The following flags opt in to stronger cleanup:

```text
--remove-textless-default-font-representatives
--remove-textless-style-representatives
--remove-textless-column-style-representatives
--remove-unreferenced-whitespace-cells
```

Unreferenced content in hidden rows or columns is removed by default because this normally does not
change the visible workbook representation. Use this conservative opt-in to retain it:

```text
--keep-unreferenced-hidden-content-cells
```

Inactive protection-only cells are controlled by one policy argument:

```text
--inactive-protection-cell-policy remove
--inactive-protection-cell-policy keep-in-data-range
--inactive-protection-cell-policy keep-all
```

The default is `remove`: when a worksheet has no active sheet protection, cells whose only relevance
comes from applyProtection/protection style are removed, including inside the visible data range.
`keep-in-data-range` retains such cells only inside the actual visible data range. `keep-all`
retains them even outside that range.

## Requirements

You need:

* Git
* Rust toolchain
* A C/C++ build toolchain
* XLSX input files

Rust is installed through `rustup`, which manages the Rust compiler and Cargo build tool.

## Setup on Ubuntu 24.04 LTS

The following instructions use Ubuntu 24.04 LTS as a common Linux distribution.

### 1. Install system packages

```bash
sudo apt update
sudo apt install -y build-essential curl git pkg-config
```

### 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When asked, choose the default installation.

Then activate Rust in the current terminal:

```bash
source "$HOME/.cargo/env"
```

Check that Rust and Cargo are available:

```bash
rustc --version
cargo --version
```

### 3. Clone the repository

```bash
git clone <REPOSITORY_URL>
cd excel-shrink-rust
```

Replace `<REPOSITORY_URL>` with the actual Git repository URL.

### 4. Build the project

For a normal debug build:

```bash
cargo build
```

For a faster release executable:

```bash
cargo build --release
```

The release executable will be created at:

```text
target/release/excel-shrink-cli
```

### 5. Run the tests

```bash
cargo test --workspace
```

This runs the unit tests in the workspace.

For release-mode validation:

```bash
cargo test --release --workspace
```

The corpus-backed smoke regressions are opt-in because they download or read larger XLSX files:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke
```

Run one regression case directly when debugging:

```bash
cargo run --release -p excel-shrink-cli -- regression smoke --case-id trailing-layout-rows
```

The same smoke cases are also available as `corpus-regression-tests`-gated Rust tests for debugger-friendly execution.

```bash
cargo test -p excel-shrink-cli --bin excel-shrink-cli --features corpus-regression-tests corpus_regression_default_smoke_suite -- --nocapture
```

## Running Excel Shrink

### Process one XLSX file

```bash
cargo run --release -- path/to/input.xlsx path/to/output_directory
```

Example:

```bash
cargo run --release -- data/example.xlsx output
```

This writes the processed file to:

```text
output/example.xlsx
```

### Process all XLSX files in a directory

```bash
cargo run --release -- path/to/input_directory path/to/output_directory
```

Example:

```bash
cargo run --release -- data/xlsx_files output
```

### Process a directory recursively

```bash
cargo run --release -- path/to/input_directory path/to/output_directory --recursive
```

### Overwrite existing output files

By default, existing output files are skipped. To overwrite them:

```bash
cargo run --release -- data/xlsx_files output --recursive --force-overwrite
```

### Run single-threaded

For easier debugging or profiling:

```bash
cargo run --release -- data/xlsx_files output --recursive --disable-multiprocessing
```

### Change the XML chunk size

```bash
cargo run --release -- data/xlsx_files output --recursive --chunk-size 8388608
```

This example uses an 8 MiB chunk size.

### Resolve relative paths from a specific directory

When Excel Shrink is started by another tool, relative paths are normally resolved from that
process working directory. Use `--working-directory` to make relative input, output, log, and
analysis CSV paths resolve from a known workspace directory:

```bash
cargo run --release -- data/xlsx_files output --analysis-csv analysis.csv --working-directory /path/to/workspace
```

### Write cleanup analysis CSV

Use `--analysis-csv` to write one row per cleaned workbook or sheet XML entry and cleanup cause:

```bash
cargo run --release -- data/xlsx_files output --analysis-csv analysis.csv
```

Each row contains the cleanup `cause`, the exact per-cause XML byte reduction in
`reduction_bytes_uncompressed_xml`, and `reduction_ratio`, which divides that per-cause reduction by
the original sheet or workbook XML size. The ZIP-compressed XLSX file size is not used for these
analysis rows.

## Setting up WSL on Windows

These instructions use Ubuntu under Windows Subsystem for Linux.

### 1. Install WSL

Open PowerShell as Administrator and run:

```powershell
wsl --install -d Ubuntu-24.04
```

Restart Windows if requested.

Then open “Ubuntu 24.04” from the Start menu and create a Linux username and password.

### 2. Update Ubuntu inside WSL

Inside the Ubuntu terminal:

```bash
sudo apt update
sudo apt upgrade -y
```

### 3. Install required packages

```bash
sudo apt install -y build-essential curl git pkg-config
```

### 4. Install Rust inside WSL

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Check the installation:

```bash
rustc --version
cargo --version
```

### 5. Access Windows files from WSL

Windows drives are mounted under `/mnt`.

For example:

```text
C:\Users\YourName\Documents
```

becomes:

```text
/mnt/c/Users/YourName/Documents
```

A path on drive `R:` becomes:

```text
/mnt/r/...
```

Example command:

```bash
cargo run --release -- \
  /mnt/r/excel-shrink-paper/scripts/scrape_destatis_xlsx_files/downloaded \
  /mnt/r/excel-shrink-paper/scripts/scrape_destatis_xlsx_files/downloaded_shrunk \
  --recursive
```

### 6. Recommended WSL workflow

For best performance, clone and build the Rust project inside the Linux file system, for example:

```bash
mkdir -p ~/repositories
cd ~/repositories
git clone <REPOSITORY_URL>
cd excel-shrink-rust
cargo build --release
```

Input and output data may still be stored on a Windows drive such as `/mnt/r/...`, but building the Rust project itself is usually faster and more reliable inside the WSL Linux home directory.

## VS Code usage

This repository contains VS Code configuration files in `.vscode/`.

### Recommended extensions

Install the following extensions:

* **Rust Analyzer** (`rust-lang.rust-analyzer`) – Rust language support, navigation, refactoring, and code intelligence
* **CodeLLDB** (`vadimcn.vscode-lldb`) – Native Rust debugging support
* **WSL** (`ms-vscode-remote.remote-wsl`) – Seamless development inside WSL
* **Even Better TOML** (`tamasfe.even-better-toml`) – Improved editing support for `Cargo.toml`
* **Error Lens** (`usernamehw.errorlens`) – Displays compiler errors directly in the editor
* **GitLens** (`eamodio.gitlens`) – Enhanced Git integration and code history

### Opening the project

1. Install Visual Studio Code.
2. Install the recommended extensions listed above.
3. Open the repository from inside WSL using the VS Code WSL integration.
4. Wait for Rust Analyzer to finish indexing the workspace.
5. Build the project:

```bash
cargo build
```

### Navigation and code exploration

Rust Analyzer provides powerful navigation features:

* **Go to Definition**: `F12`
* **Peek Definition**: `Alt+F12`
* **Go to Declaration**: Right-click → *Go to Definition*
* **Find All References**: `Shift+F12`
* **Rename Symbol**: `F2`
* **Go to Implementation**: Right-click → *Go to Implementations*
* **Call Hierarchy**: Right-click on a function → *Show Call Hierarchy*
* **Type Hierarchy**: Available for supported Rust types and traits
* **Outline View**: Quickly navigate modules, structs, enums, traits, and functions

These features are especially useful when tracing execution paths through `pipeline.rs`, `sheet.rs`, and `workbook.rs`.

### Debugging

For a timeline of parallel cleanup work and waits, run the VS Code task
**Profile smoke test (pick case, Perfetto, no debugger)**. It writes a unique trace
under `.tmp/profiles/` for [Perfetto](https://ui.perfetto.dev). See the
[profiling guide](tools/profiling/README.md) for interpreting wall times, buffer
counters, CPU sampling, and repeatable performance comparisons.

For an optimized regression run **without a debugger**, use **Terminal > Run Task**:

* **Release smoke test (pick case, no debugger)** runs one selected smoke case using
  its baseline cleanup settings and checks. This is the short functional check.
* **Release smoke visual profile matrix (pick case, no debugger)** runs the selected
  case across all visual profiles, including Excel recalculation and screenshots.

Both tasks build automatically with `cargo run --release`. They do not start CodeLLDB.
The first run may include dependency compilation and downloading the test workbook.
The JSON regression report records `shrink_duration_ms` for the cleaning pipeline
separately from those setup costs and subsequent validation. Some smoke cases enable
analysis to check cleanup causes, so their timings include that analysis overhead.
Use repeated runs with the same case and settings when comparing performance.
The matrix's total duration also includes Excel and screenshot work and is not a
measurement of cleaning speed alone.

The repository includes VS Code debug configurations in `.vscode/launch.json`.

Install the recommended CodeLLDB extension in the environment where the repository is
opened (for WSL, install it in WSL). Launch configurations build through Cargo and use
the executable path reported by Cargo, including the Windows `.exe` suffix or a custom
Cargo target directory.

Launch configurations and build tasks add the current user's standard Rustup directory
(`~/.cargo/bin`, or `%USERPROFILE%\.cargo\bin` on Windows) to the inherited PATH.
This handles a VS Code extension host started before Rust was installed: running
`source "$HOME/.cargo/env"` in a terminal only updates that terminal's environment.
For a nonstandard Cargo installation, put its bin directory on VS Code's PATH before
starting VS Code. No personal absolute paths are required in the repository.

The workspace's `.vscode/settings.json` also adds the Rustup bin directory to the
debugger's own environment before it loads Rust formatters. Cargo's build environment
and the debugger's environment are separate. On Unix, the debugger passes `SIGCHLD`
through without stopping or notifying: the regression runner starts child processes,
and their exit notifications should not interrupt debugging. Other signals retain
their normal debugger behavior.

Cleanup configurations prompt for input and output paths, relative to the workspace
unless absolute paths are entered. For regression configurations, leave the optional
corpus root empty to use the cache, sibling corpus, or configured download URL.
The visual profile matrix additionally requires Windows Excel and PowerShell access
for recalculation and screenshots. Rust source (`rustup component add rust-src`) is
useful for source navigation but does not fix Cargo executable discovery.

Useful debugging features include:

* Set breakpoints by clicking in the editor gutter
* Conditional breakpoints
* Watch expressions
* Variable inspection
* Call stack inspection
* Step Into (`F11`)
* Step Over (`F10`)
* Step Out (`Shift+F11`)
* Restart and hot-reload debug sessions

For easier debugging, consider running the CLI with:

```bash
cargo run -- path/to/input.xlsx output
```

or

```bash
cargo run -- path/to/input_directory output --disable-multiprocessing
```

### Running tests from VS Code

Rust Analyzer automatically detects Rust tests.

You can:

* Click **Run Test** above a test function
* Click **Debug Test** above a test function
* Run all tests from the Testing panel
* Run tests from the terminal:

```bash
cargo test
```

Run a specific test:

```bash
cargo test test_name
```

Run tests with output:

```bash
cargo test -- --nocapture
```

### Useful Rust Analyzer commands

Open the Command Palette (`Ctrl+Shift+P`) and try:

* Rust Analyzer: Reload Workspace
* Rust Analyzer: Restart Server
* Rust Analyzer: View Syntax Tree
* Rust Analyzer: Show References
* Rust Analyzer: Expand Macro Recursively

These commands are particularly useful when investigating macro-generated code or workspace indexing issues.

## Reproducing paper experiments

A typical reproduction workflow is:

```bash
git clone <REPOSITORY_URL>
cd excel-shrink-rust
cargo build --release
cargo test
mkdir -p output
cargo run --release -- path/to/xlsx_corpus output --recursive
```

After the run, the output directory contains the processed XLSX files. These files can then be compared with the original input files, for example by measuring file sizes before and after processing.

A simple Linux command for comparing file sizes is:

```bash
du -b path/to/xlsx_corpus/*.xlsx
du -b output/*.xlsx
```

For recursive corpora:

```bash
find path/to/xlsx_corpus -name "*.xlsx" -printf "%s %p\n" > original_sizes.txt
find output -name "*.xlsx" -printf "%s %p\n" > cleaned_sizes.txt
```

## Troubleshooting

### `cargo: command not found`

Rust is not installed or the current terminal does not know where Cargo is installed.

Run:

```bash
source "$HOME/.cargo/env"
```

If that does not help, reinstall Rust with `rustup`.

### Permission problems on Windows-mounted drives

If you process files under `/mnt/c`, `/mnt/d`, or `/mnt/r`, make sure the files are not open in Excel or another program.

If problems continue, copy a small test corpus into the WSL home directory and run the tool there:

```bash
cp -r /mnt/r/path/to/xlsx_files ~/xlsx_test
cargo run --release -- ~/xlsx_test ~/xlsx_test_output --recursive
```

### Existing output files are skipped

Use:

```bash
--force-overwrite
```

### The program finds no XLSX files

Check that the input path exists and that the files end with `.xlsx`.

For recursive directory search, use:

```bash
--recursive
```

### Rust Analyzer does not detect the workspace

Try:

```bash
cargo check
```

Then reload VS Code and run:

```text
Rust Analyzer: Reload Workspace
```

from the Command Palette.

### The build is slow

Use the release build only when needed:

```bash
cargo build
```

For final performance runs, use:

```bash
cargo build --release
```

## Notes for reviewers

This repository is intended to support reproducibility of the accompanying paper. The goal is not to require reviewers to know Rust in advance. The essential commands are:

```bash
cargo build --release
cargo test
cargo run --release -- <INPUT_XLSX_OR_DIRECTORY> <OUTPUT_DIRECTORY> --recursive
```

The Rust package manager Cargo automatically downloads the required Rust dependencies listed in `Cargo.toml` and locked in `Cargo.lock`.
