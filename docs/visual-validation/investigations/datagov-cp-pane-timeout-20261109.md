# Data.gov Check Performance pane-discovery timeout

The separate Data.gov preparation encountered its first failure at source
occurrence 401, `2016-nyc-school-survey__76fb9635-5e40-48d5-928f-2079026566cc_69cd96c983e4.xlsx`.
Its preserved baseline is 61,998,296 bytes. Native opening and saving completed,
leaving `original-saved.xlsx` and `before-optimization.xlsx`. The failure occurred
during optimization when UI Automation `FindAll` timed out with HRESULT
`0x80131505`. It was recorded as `excel_automation_failed`, without a successful
optimization artifact. The earlier opening phase must not be reported as the
cause of this specific failure.

Pane discovery searched the complete Excel accessibility subtree for an
`MsoWorkPane` class. The revised adapter first enumerates native child HWNDs of
the exact owned Excel window. It checks process ownership and the root ancestor,
then converts only matching native pane hosts to accessibility elements. The
existing localized-title match, unique-action checks, bounded pane traversal and
positive completion requirements remain in place. Providers without a native
pane handle retain their previous accessibility fallback.

## Focused acceptance

The hidden-control test `native-performance-panes.tests.ps1` compiled the C#
helper and verified an exact owned child match, no match for an absent class,
and rejection of a different owner PID. No test window was displayed and no
global input was sent.

The known small `01011970_Auszug_GV.xlsx` preparation succeeded with the revised
adapter. A second positive control additionally recorded
`Pane lookup owned_native_handles`, proving that it exercised the new lookup
path. Its trace observed the pane about 0.139 seconds after the native command
returned. This observation is not a controlled speedup estimate. The preparation
receipt SHA-256 is
`17f5e15843bdc0ca0ebf60f8cc82ef763529f9fc5f572e8a02b6fa2776d3fde0`.

Evidence remains under the private
`G:\SheetShrink-Night-20261109\cp-recovery\uia-observed-control` directory.
The experiment runtime was separately packaged and verified as 114 files.
The ongoing Data.gov preparation retains its previous immutable runtime.

## Large-file retry

An independent single-input retry preserves occurrence 401's exact source
identity and uses a new output directory. During observation its Excel process
remained responsive and accumulated approximately one CPU-second per elapsed
second. The read-only owned-desktop dialog observer found no visible repair or
security dialog at the sampled instant. The retry reached the optimization
phase and observed the performance pane. It terminated after 305.2709567 seconds
with another `excel_automation_failed` result. This time the provider timed out
in `AutomationElement.FromHandle`, with the same inner HRESULT `0x80131505`.
The direct-handle lookup therefore does not resolve the large-file failure.
Its preparation receipt is retained under `uia-timeout-retry`; neither this
failure nor the original can be counted as prepared. The evidence distinguishes
an expensive complete-tree search from broader accessibility-provider
unavailability while Excel is CPU-active. A bounded retry policy for specifically
transient provider failures still needs investigation. Unknown native outcomes
must continue to fail explicitly.
