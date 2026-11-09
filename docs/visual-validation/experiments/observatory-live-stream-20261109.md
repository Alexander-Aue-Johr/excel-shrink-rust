# Incremental Observatory status transport

The local HTTP measurements on 2026-11-09 returned 6,880,603 bytes for the full
state, 11,299 bytes for its live projection and 1,771,351 bytes for the retained
capture comparisons. The measurements are individual UTF-8 response sizes.

A bounded native host test used 100 synthetic completed comparison cards, each
with a 16,000-character retained detail. A Node WebSocket client independently
applied the browser's patch reducer. After the baseline it changed exactly one
fixture card, checked that the next patch contained exactly one comparison
operation, and checked that the following patch contained none.

| Message | Bytes |
| --- | ---: |
| Initial snapshot | 1,627,309 |
| One changed comparison and status | 17,722 |
| Status with unchanged comparisons | 1,415 |

The first production attempt exposed timestamp-prefixed comparison IDs that the
pure-UUID synthetic fixtures missed. The protocol now accepts both recorded ID
forms, with explicit Rust and JavaScript regression checks. A subsequent real
Enron stream delivered 1,838,826 bytes initially, followed by patches of 82,035
and 42,223 bytes. These are observed samples, not a sustained throughput result.

The real-client check also exposed an unclean close handshake. Flushing the
protocol close reply passed a separate isolated native test with close code 1000
and wasClean=true. Test hosts were stopped after their checks. The Enron runner
and four comparison groups continued throughout host replacement.

The stream reduces repeated transfer and browser parsing of unchanged cards.
The server still samples its bounded registry every three seconds. Detailed
selected-workbook and image requests retain their existing HTTP endpoints.
Dynamic warm-worker capacity expansion is separate outstanding work.
