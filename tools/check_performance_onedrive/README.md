# Run Check Performance on a OneDrive folder

`tools/check-performance-onedrive.py` is the stable command-line entry point.
The browser workflow, state machine, and SQLite progress store live together in
`runner.py`; `pane.js` observes Excel's Performance pane. The runner opens each XLSX directly from a shared OneDrive
folder, runs **Review → Performance → Optimize all** if offered, waits for
Excel's **Saved/Gespeichert** indicator, and returns to the folder in the same
tab. It does not upload or download workbooks.

Use Windows Python from the repository root. Edge and Playwright are required;
`psutil` adds browser RAM measurements. For example:

```powershell
python -m pip install playwright psutil
python tools/check-performance-onedrive.py 'YOUR_EDITABLE_ONEDRIVE_FOLDER_LINK' --headed
```

Sign in in the browser window on the first run. The browser profile is retained
under `.tmp/onedrive-check-performance/browser-profile`. Later runs can omit
`--headed`. If the folder's top bar says **Sign in** or Microsoft redirects to
a sign-in page, a visible run starts or waits for authentication before reading
the file list. The runner follows the sign-in link in its existing tab, and
OneDrive workbook navigation also stays there. On restart, Chrome's saved tab
history is discarded so a fresh folder tab opens; cookies and other sign-in
data stay in the profile.
A headless run stops with an instruction to
restart using `--headed`. To reuse a profile from the earlier web experiment, pass
`--profile-dir .tmp/check-performance-web/edge-profile`. Do not run two scripts
against the same browser profile at once. `--browser-channel chrome` or
`--browser-channel chromium` can be used when Edge is unavailable; install the
chosen Playwright browser first if needed.

The tool can also run entirely in WSL: it uses Excel for the web and has no
Excel COM dependency. Install Linux Playwright and Chromium in a WSL Python
environment, then sign in once in a visible WSLg browser window:

```bash
python3 -m venv .tmp/onedrive-check-performance/venv
source .tmp/onedrive-check-performance/venv/bin/activate
python -m pip install playwright psutil
python -m playwright install chromium
python tools/check-performance-onedrive.py 'YOUR_EDITABLE_ONEDRIVE_FOLDER_LINK' --browser-channel chromium --headed
```

Subsequent WSL runs can omit `--headed`. Windows and WSL use different browser
profiles and cannot share the existing Windows login session. The first WSL
sign-in therefore has to be done once in the Linux browser. Headless WSL runs
do not need a visible desktop; the initial `--headed` run needs WSLg or another
graphical display.

If WSLg does not show the browser window, run the visible first sign-in with
Windows Python instead. Its profile is separate, so use Windows Python for the
following unattended runs as well. A signed-out headless profile fails promptly
with `Microsoft sign-in required` and does not mark any workbook as processed.

On the first run, the script discovers XLSX files by scrolling OneDrive's list.
Later runs reuse the known names from SQLite and skip that full discovery pass.
It sorts the folder by modification time, oldest first, and opens the next
known unfinished file directly from the visible rows. Optimized files can move
to the bottom of the view; the runner checks the current rows again before
scrolling and wraps to the top when necessary. It updates
`.tmp/onedrive-check-performance/<folder-id>/progress.sqlite3` after each phase.
This ignored database records file names, attempts, status, outcomes, UTC times,
elapsed and per-phase seconds. It samples the Excel-frame JavaScript heap and
the working set of all processes using this browser profile after each file
finishes, so the measurement does not delay the Optimize all click. The
**profile working set is not
the physical RAM of one tab**: Chromium shares processes across tabs. If
`psutil` is unavailable, that field is `null`. The editable folder URL is not
stored in the progress file; `<folder-id>` is its hash.

In an interactive terminal, the runner shows one row per finished workbook with
seconds spent opening Excel (`O`), selecting Review (`R`), opening Performance
(`P`), optimizing or checking (`X`), and confirming the cloud save (`S`). A
single status line below the rows updates the current file, processed count,
remaining count, and approximate time remaining. The estimate uses the median
of up to 20 recently completed workbooks; a corpus with very different file
sizes can take substantially longer or shorter. Before any timing data exists,
the estimate says `unknown`. Redirected output keeps the detailed phase log.
If Excel keeps the Performance command disabled, the workbook is marked
`skipped` with outcome `performance_unavailable`; it is not counted as a
successful optimization. The status line reports skipped files separately.
For Excel's specific read-only warning about many invalid objects, the runner
clicks **Remove and Edit** first. Excel removes those objects from the OneDrive
copy; the run records `invalid_objects_removed` and adds
`_after_remove_and_edit` to the outcome. These files therefore contain a
separate Excel repair in addition to any Check Performance change. Other
read-only warnings are not accepted automatically.
If Excel displays **Couldn't Open the Workbook**, even over a visible ribbon,
the runner returns to the folder and tries opening the file once more. A second
explicit rejection is recorded as `skipped` with outcome
`workbook_cannot_open`; Check Performance is not reported as successful.
Use `--retry-skipped` to revisit it after the workbook or service changes.
Excel can also show an **Open in Desktop App?** dialog saying the workbook is
protected by Information Rights Management (IRM) and cannot be edited in a
browser. The Review ribbon may already be visible behind this dialog. The
runner recognizes the IRM message, records `skipped` with outcome
`irm_browser_edit_unavailable`, and continues with the next workbook. It does
not click **Open in Desktop App** or run Check Performance on that copy.
This was observed with `auslaend-bevoelkerung-2010200217005.xlsx`; the
browser restriction alone does not establish whether desktop Excel can edit
the workbook. Use `--retry-skipped` only after its permissions or format have
changed.

On the first run with this version, existing `progress.json` data is imported
into SQLite, including completed files and timing events. The importer leaves
the JSON file in place so it can be checked before removal; subsequent progress
is recorded only in SQLite. The current OneDrive folder's JSON was checked
against the SQLite database and removed after migration on 3 November 2026.

Re-run the same command after an interruption. Completed and skipped files are
not reopened.
An unexpected workbook error records a failed attempt, closes the browser and
Playwright driver, and retries that workbook once in a new browser window
using the retained sign-in profile. If the second attempt fails, the file is
marked `skipped` with outcome
`error_after_retry`; both attempt errors remain in SQLite. The runner then
opens a fresh browser and continues with the next file. It exits nonzero after
the batch so those skips are not mistaken for complete success. A skipped
workbook may have been partly optimized before the error; inspect it before
using `--retry-skipped`.

If OneDrive redirects to `Throttle.htm`, the folder view times out, or the
browser connection closes, the runner starts a new Playwright driver and
browser. Folder startup is
retried up to nine times with increasing delays. A missing interactive sign-in
or a folder that still cannot be opened stops the run without marking unrelated
files as skipped. Browser cleanup errors are logged without replacing the
original workbook error. An interrupted file is reopened on the next run; the
tool closes its previous attempt as interrupted before starting a new one. Use
`--refresh-list` to scan for files
uploaded after the initial discovery. OneDrive can upload new rows while
scanning; refresh again after all uploads finish to pick up late arrivals.

Useful small runs:

```powershell
python tools/check-performance-onedrive.py 'YOUR_LINK' --list-only
python tools/check-performance-onedrive.py 'YOUR_LINK' --refresh-list
python tools/check-performance-onedrive.py 'YOUR_LINK' --max-files 1
python tools/check-performance-onedrive.py 'YOUR_LINK' --only-file 'example.xlsx'
python tools/check-performance-onedrive.py 'YOUR_LINK' --only-file 'example.xlsx' --retry-skipped
```

`--list-only` refreshes names without opening workbooks. `--max-files` limits the
number processed in this invocation; the remaining files are left pending.
`--retry-skipped` rechecks previously unavailable files, including one that
was repaired manually after an earlier run.
The command exits unsuccessfully if a file fails twice or the folder cannot be
recovered. A timeout is never recorded as optimization
success. This browser flow was tested with Edge and
Excel for the web on 3 November 2026; it depends on Microsoft's current UI.
In that test, one workbook reached Excel's cloud-save confirmation in 38.5 s.
The next invocation skipped that workbook without opening it. Its recorded
Excel-frame JavaScript heap was about 207 MB when the editor became ready;
the whole browser profile used about 2.0 GB of working-set RAM at that point.

The Performance pane uses a `MutationObserver` to react to button and result
changes. The progress log distinguishes a dispatched click from a button state
change that confirms Excel accepted it. An unchanged button after 20 seconds
is an error, not an optimization in progress. The pane observer is covered by
isolated Chromium tests in `test_pane.py`.

The workbook status follows a small state machine:
`pending → opening → checking → optimizing → saving → completed`. A workbook
that cannot be checked goes to `skipped` from any unfinished stage; `failed`
records a retryable attempt after a workbook error or throttling. An already-clean workbook
goes from `checking` directly to `completed`.
The `optimizing` state begins only when Excel changes
the button or reports completion. Existing progress files with the old
`running` status remain resumable.

Editor readiness starts an explicit Excel sign-in as soon as its control appears,
even if Review is not visible yet, and then waits for Review. It does not require an account
button, whose label varies between Excel sessions. Review is considered
selected only when its
`aria-selected` attribute changes. These waits target the relevant controls;
they do not wait for the whole Excel DOM to become quiet. A fixed 50 ms delay
for every one of 30,000 files would already add 25 minutes, even when Excel
was ready immediately. A Review click is retried only if its selected state
does not appear within two seconds.
