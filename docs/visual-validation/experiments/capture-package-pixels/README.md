# Combined capture-package pixel budgets

This experiment captures the same eight real Lysimeter ranges in one isolated
production Office.js resident. Each range estimates 29.20 MP using the existing
conservative geometry rule. The image limit remains 32 MP and 30,000 pixels per
edge. Only the combined allowance for multiple images changes.

Two passes run in opposite order after one warm-up image. All decoded RGBA
pixels and dimensions matched for every range in every pass.

| Package allowance | Images per package | Mean seconds |
| --- | --- | --- |
| 32 MP | 1 | 5.584 |
| 64 MP | 2 | 5.520 |
| 128 MP | 4 | 4.983 |
| 256 MP | 8 | 4.741 |

Timings include transport, PNG decoding and SHA-256 verification in Python.
The largest sampled owned-job memory was 865 MB. Samples read the existing
resident metrics every 250 ms; the publisher's own cadence limits peak
precision. This is a single-workbook experiment with compressible content,
not a corpus-wide optimum or a guaranteed upper memory bound.

The production API now accepts `capture_batch_megapixels` from 32 through 256,
independently of `capture_batch_ranges` and `geometry_batch_ranges`. The default
remains 32 MP for compatibility. Controls are reloaded at package boundaries.
The Enron trial can use eight ranges and 256 MP, then reduce either value if
native memory or rendering failures appear. Each individual export retains its
existing geometry validation and native failure handling.

Run from the repository root, choosing a new output directory on V:.

```powershell
python docs/visual-validation/experiments/capture-package-pixels/run.py `
  --output V:\validation-experiments\capture-package-pixels `
  --binary V:\path\to\excel-shrink-cli.exe `
  --workbook V:\path\to\lysimeter-original-rebuilt.xlsx
```

The source workbook is copied by the existing resident launcher. It is never
saved in place. The experiment closes its owned worker in a finally block.
