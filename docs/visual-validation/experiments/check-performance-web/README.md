# Check Performance in Excel for the web

This isolated experiment checks whether the browser version can optimize a
synthetic XLSX on a computer whose desktop Excel does not expose the
`CheckPerformance` ribbon command. It does not change the corpus runner.

## Run

Run with **Windows Python** from the repository root:

```powershell
python docs/visual-validation/experiments/check-performance-web/run.py --output .tmp/check-performance-web
```

The experiment opens a separate Edge profile under the output directory. Sign
in to your personal Microsoft account in that window if prompted. If Microsoft
shows signed-in account tiles, the experiment asks which one it may reuse and
remembers a keyed SHA-256 fingerprint of the chosen address. It asks again if
that account no longer appears. The Edge profile itself contains session data
and must remain private. The script then uploads
`excel-shrink-check-performance-probe.xlsx` to OneDrive, opens it
in Excel for the web, selects **Review → Performance → Optimize all**, and
downloads a copy. The browser profile retains the login for a later run. Do
not commit or share that profile. The uploaded cloud workbook remains in your
OneDrive until you remove it.

The fixture has a real value `A1 = 42` and a formatted, empty `Z10000`. The
download is a success only if its sheet XML still has `A1 = 42` and no longer
contains `Z10000`. `result.json` records the before/after checksums, sizes,
and cell observations. `browser-result.json` records browser-phase failures.
The downloaded workbook stays under `downloads/` for inspection. No account
tokens, cookies, workbook URL, or browser profile data are written to either
result JSON file.

The script uses Python Playwright with the installed Edge. Install Playwright
into the experiment's ignored output directory before the first browser run:

```powershell
python -m pip install --target .tmp/check-performance-web/python-packages playwright==1.64.1
```

The browser closes when the run finishes. If a control changes, the probe
fails at the named step instead of claiming optimization. Reusing a previously
approved account does not ask again on every run. Use `--fresh-profile` to
remove the experiment's Edge profile and account choice before starting over.
Playwright supplies the upload file directly to the browser's file chooser;
the Windows Open dialog does not need manual input.

## Observed result

On 3 November 2026, the complete script ran with the signed-in Edge profile.
Excel for the web reported that it optimized two cells. The downloaded XLSX
retained `A1 = 42` and no longer contained `Z10000` in its sheet XML. The
download was 8,089 bytes; the synthetic source was 2,018 bytes. The larger
download size reflects other rewrites by Excel and is not a measure of failed
optimization. The ignored `result.json` and downloaded workbook hold the
machine-readable observation.

## Scope

This tests the cloud command and downloaded artifact on a small workbook. It
does not establish desktop/cloud equivalence or corpus throughput. Excel for
the web can rewrite a workbook while opening and saving it, independently of
Check Performance. A later controlled baseline download without optimization
would be needed to attribute every package change to the command.
The browser flow was verified with Edge; other Playwright browsers have not
been tested against this Microsoft page.

## Eight Destatis files

For a local Windows run, place the eight original XLSX files in
`.tmp/check-performance-web/destatis-input/` and run:

```powershell
python docs/visual-validation/experiments/check-performance-web/batch.py --output .tmp/check-performance-web
python docs/visual-validation/experiments/check-performance-web/summarize.py .tmp/check-performance-web/batch-result.json
```

The batch uses one headless browser context. It opens a new tab per workbook,
first one after another and then all eight at once. Browser startup is measured
once; each tab's creation, home page, upload-to-workbook URL, Review tab,
Performance pane, optimization, and download have separate timestamps. The
workbook URL and first visible ribbon are **not** proof that Excel has finished
loading; the Review timing includes waiting until its tab actually becomes
selected. `batch-result.json` contains raw wall-clock timestamps and statuses.
Each new run stores downloads under its own timestamped directory and retains
the prior result JSON. For an older run, rely on its recorded status and
download path; a file left by an earlier attempt does not count as completed.

On 3 November 2026, all eight sequential uploads, optimizations and downloads
completed in **284.8 seconds**. In the parallel pass, seven downloads completed
in **79.4 seconds**; their matching sequential jobs took **271.5 seconds** in
total, a **3.42×** speedup for those seven. Up to seven upload/open intervals
and five optimization intervals overlapped. The longest of the seven sequential
jobs took **82.8 seconds**, close to the seven-file parallel makespan.

The eighth parallel job, `krankenhausverzeichnis-3500100227005.xlsx`, reached
**Optimize all** but did not show completion after more than five minutes. Its
sequential optimization had completed in under one second. The batch was
stopped and its raw state saved as `batch-result-eight-tabs-interrupted.json`;
`batch-interruption.json` records the observed waiting time. Thus the test
demonstrates real overlap and a speed gain for seven completed files, but **does
not establish that eight simultaneous jobs finish in the time of the slowest
sequential file**. This is one run on one machine and one cloud session; uploads
also create new workbook copies in OneDrive.

To verify the no-work case separately, run:

```powershell
python docs/visual-validation/experiments/check-performance-web/run.py --output .tmp/check-performance-web --already-clean --headless
```

Excel reported **Keine Optimierung erforderlich** for the synthetic clean
workbook; the experiment recorded `already_clean` and downloaded the workbook.

To create and inspect only the local fixture, run with `--prepare-only`.

## Existing OneDrive copies: staged workers

`onedrive_staged.py` opens workbooks already present in copied OneDrive folders.
It compares three policies: complete each file before starting the next
(`sequential`), open all selected files and optimize them together (`parallel`),
or open them together but optimize one at a time (`hybrid`). The staged modes
wait until **all** editors are ready before Review, until all Review tabs are
selected before Performance, and until all panes are ready before optimization.
They pause for 5, 3, and 3 seconds between those stages. Each mode uses its own
copied folder so an earlier optimization cannot make a later test appear fast.
The script records per-file phase times and the visible outcome. It does not
download the optimized files. Once Excel reports success, it waits for the
cloud indicator to say **Gespeichert** before closing the tab.

Put the editable OneDrive folder URL in the ignored file
`.tmp/check-performance-web/onedrive-share-url.txt`, then run one mode per
browser process with Windows Python:

```powershell
python docs/visual-validation/experiments/check-performance-web/onedrive_staged.py --output .tmp/check-performance-web --mode sequential
python docs/visual-validation/experiments/check-performance-web/onedrive_staged.py --output .tmp/check-performance-web --mode parallel --limit 4
python docs/visual-validation/experiments/check-performance-web/onedrive_staged.py --output .tmp/check-performance-web --mode hybrid --limit 4
```

The default copied folders are `Optimized Kopieren4`, `Optimized Kopieren3`, and
`Optimized Kopieren2`, respectively. `--folder` selects another copied folder.
Each run writes `onedrive-staged-result.json` and backs up the previous result
in the ignored output directory. The editable share URL and Edge profile stay
outside version control. A copied folder is consumed by the experiment; use a
fresh copy for another comparison.

### Observations, 3 November 2026

| Policy | Files | Time | Outcome |
| --- | ---: | ---: | --- |
| Sequential | 8 | 178 s | 8 optimization success messages; 6 save indicators already said **Gespeichert**, 2 still said **Wird gespeichert** when read. |
| Parallel load and optimize | 8 | stopped during loading | Only 2 editors became ready promptly; several others missed the 120 s readiness limit. |
| Parallel load and optimize | 4 | 237 s | All editors ready after 38 s. Two optimizations succeeded; two timed out after 180 s. |
| Parallel load, serial optimize | 4 | 143 s | One editor missed the 120 s limit; the other three optimizations succeeded. |
| Parallel load, serial optimize, fresh copy | 4 | 323 s | One editor missed the 120 s limit; two optimizations and cloud saves succeeded; the last optimization timed out. |

For the first four sequential files, reaching the editor took about 43 s in
total. The successful four-tab parallel load took 38 s; the two hybrid loads
took about 123 s because one tab did not become ready. Review and Performance
were short relative to loading: the parallel four-tab run needed about 7 s and
6 s per stage, including the deliberate pauses. The sequential run spent about
0.2 s per file selecting Review and roughly 2–4 s opening Performance.

The evidence does not support keeping eight workbooks open and optimizing them
as a reliable throughput strategy on this machine. Serial processing was both
faster and more reliable in these runs. Parallel loading remains a possible
optimization, but needs bounded worker counts, editor readiness timeouts,
recovery of stalled tabs, and explicit save confirmation before it can replace
the serial path. A visible **Optimize all** button or a completed click alone
does not establish that optimization or cloud saving finished. These timings
include network and OneDrive service variation; they are observations from one
day, not stable performance limits.
