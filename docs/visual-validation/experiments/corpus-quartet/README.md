# Benchmark: vier Quartette, 16 residente Excel-Prozesse

`run.py` ist der Einstieg für den Vergleich von 1, 2 und 4 gleichzeitig
captureenden Gruppen. Jede der vier Gruppen enthält vier getrennte Excel-
Prozesse: Original, Shrink Default, Maximum und Too Aggressive. Der Runner
bereitet alle Shrink-Dateien und drei unterschiedliche numerische Kontrollwerte
pro Arbeitsmappe vor, bevor ein Excel-Prozess startet. Die Originaldateien
bleiben unverändert.

**Messgrenze:** Bei *jedem* der drei Läufe werden dieselben 16 Excel-Prozesse
geöffnet. Jeder Worker überprüft seinen Workbook-Namen, plant die Capture-
Kacheln und meldet sich beim Host als capture-bereit. Erst wenn alle 16 bereit
sind, startet die Uhr. Der Scheduler aktiviert 1, 2 oder 4 Gruppen und gibt
den nächsten Platz frei, sobald eine Gruppe fertig ist. Excel und Add-ins
bleiben bis zum Ende des Laufs geöffnet. Lade- und Planungszeit ist getrennt.

Innerhalb einer aktiven Gruppe capturen alle vier Worker zunächst autonom
alle Kacheln und schicken nur PNG-SHA-256, Bildgröße und Kachelbereich als
gesammelte Runde. Danach liefert der Host die abweichenden Kacheln. Diese
werden rekursiv in bis zu vier Teilbereiche (2 × 2) aufgeteilt und erneut
gehasht; erst für die eingegrenzten Unterschiede werden PNGs übertragen und
pixelgenau verglichen. Andere Gruppen warten dabei nicht auf diese Gruppe.

Standardauswahl: Kernhaushalte, Wanderungen, Stromsteuerstatistik und
Verkehrsunfälle. Erfasst wird jeweils das größte sichtbare Blatt bis Zeile
400. `--scope-map` setzt explizite Bereiche. Das ist ein reproduzierbarer
Durchsatztest, kein Zertifikat für die gesamte Arbeitsmappe.

Auf Windows mit Excel und .NET 8 SDK oder aus WSL mit Zugriff auf Windows-Excel:

```bash
python3 docs/visual-validation/experiments/corpus-quartet/run.py \
  --repo-root . \
  --corpus-root corpus-local/original_destatis_xlsx_files \
  --cli target/release/excel-shrink-cli \
  --output .tmp/corpus-quartet-benchmark \
  --repeats 2
```

Unter Windows `python` und `target/release/excel-shrink-cli.exe` verwenden.
Mit `--only <Teilname>` (viermal) lassen sich genau vier andere lokale
Korpusdateien wählen; nur diese vier werden vor dem Excel-Start geschrumpft.
`--prepared-dir` verwendet vorhandene Shrink-Versionen nur bei passendem
Manifest mit Original-, CLI-, Profil- und Datei-Prüfsummen. `--prepare-only`
erzeugt nur Dateien und Kontrolländerungen; derselbe Ausgabeordner kann danach
für den eigentlichen Lauf verwendet werden.

Excel startet standardmäßig minimiert auf dem interaktiven Windows-Desktop;
`--visible` zeigt die Fenster zur Diagnose. Der isolierte `--hidden-desktop`
ist nur ein Diagnosemodus: Im lokalen Versuch meldete sich Office.js dort
nicht zuverlässig an. Bei `--visible` gibt es eine automatisch aktualisierte
`live.html` für den Start- und Capture-Fortschritt.

Ein kleines, festes Korpusbeispiel verwendet vier `--only`-Angaben
(`seeschifffahrt`, `foerderprogramme`, `stromsteuerstatistik`, `jahresbericht`)
mit [small-corpus-scopes.json](small-corpus-scopes.json). Eine Scope-Grenze
direkt vor belegten Nachbarzellen wird vor dem Excel-Start abgelehnt, weil die
Überlaufprüfung sonst keine sichere freie Zielspalte hat.

```bash
python3 docs/visual-validation/experiments/corpus-quartet/run.py \
  --repo-root . \
  --corpus-root corpus-local/original_destatis_xlsx_files \
  --cli target/release/excel-shrink-cli \
  --output .tmp/corpus-quartet-small \
  --only seeschifffahrt --only foerderprogramme \
  --only stromsteuerstatistik --only jahresbericht \
  --scope-map docs/visual-validation/experiments/corpus-quartet/small-corpus-scopes.json \
  --repeats 2
```

## Wiederholung auf dem Tower

In einem vollständigen Checkout auf Windows, mit angemeldetem Desktop-Excel,
Python 3.12+, PowerShell und .NET 8 SDK, vom Repository-Root aus:

```powershell
cargo build --release -p excel-shrink-cli
$bench = 'docs/visual-validation/experiments/corpus-quartet/run.py'
$common = @('--repo-root', '.', '--corpus-root', 'corpus-local/original_destatis_xlsx_files', '--cli', 'target/release/excel-shrink-cli.exe', '--repeats', '3', '--ignore-memory-guard')
$tag = Get-Date -Format 'yyyyMMdd-HHmmss'
$small = ".tmp/corpus-quartet-tower-small-$tag"
$priority = ".tmp/corpus-quartet-tower-priority-$tag"
$smallSelection = @('--only', 'seeschifffahrt', '--only', 'foerderprogramme', '--only', 'stromsteuerstatistik', '--only', 'jahresbericht', '--scope-map', 'docs/visual-validation/experiments/corpus-quartet/small-corpus-scopes.json')
python $bench @common --output $small @smallSelection --prepare-only
python $bench @common --output $priority --prepare-only
$started = Get-Date; python $bench @common --output $small @smallSelection; ((Get-Date) - $started).TotalSeconds | Set-Content "$small/wall-seconds.txt"
$started = Get-Date; python $bench @common --output $priority; ((Get-Date) - $started).TotalSeconds | Set-Content "$priority/wall-seconds.txt"
```

Die erste Messung wiederholt die kleine Laptop-Auswahl. Die zweite erfasst
Kernhaushalte, Wanderungen, Stromsteuerstatistik und Verkehrsunfälle. Beide
Ausgabeordner mit `report.html`, `results.json` und `measurements.csv` behalten;
auch fehlgeschlagene Läufe gehören zur Auswertung. `wall-seconds.txt` misst
den gesamten Lauf nach dem Shrinken, einschließlich Excel-Start; der Speedup
im Report beginnt erst an der gemeinsamen Startlinie. Lokale Korpusdateien werden nicht durch
Git übertragen. Ein flacher Clone braucht den vollständigen Verlauf für den
fest referenzierten Office-Capture-Commit.

Vor jedem Lauf schätzt der Runner den Bedarf aller 16 Prozesse anhand der
Dateigrößen und verlangt standardmäßig 2 GiB freien RAM als Reserve. Der
Launcher prüft freien physischen RAM während Start und Capture erneut.
Überschrittene Limits und unvollständige Startlinien ergeben einen
fehlgeschlagenen oder `resource-limited` Lauf, niemals einen Speedup-Wert.
Die Schätzung kann den tatsächlichen Excel-Bedarf nur grob abbilden.
Für einen ausdrücklich gewünschten Versuch ohne diese Speichergrenzen gibt es
`--ignore-memory-guard`; der Lauf wird in `results.json` entsprechend markiert.

`results.json` enthält Rohdaten, `measurements.csv` die Capture-, Hash-,
Übertragungs-, Nachprüfungs-, CPU- und RAM-Messwerte, `report.html` Laufzeiten,
Speedup und Bildbelege. Die Messung endet, sobald alle vier Gruppen ihre
Vergleiche und PNG-Übertragungen an den Host abgeschlossen haben; das Kopieren
der Belege in den Reportordner erfolgt danach. `controls.json`, `plans.json`
und `provenance.json` dokumentieren die Eingaben. Die pro Gruppe geschriebenen
Logs und `result.json` helfen bei Add-in-Startproblemen.

Der Host-Protokolltest liegt unter `tests/run.ps1`.

## Laufzeitstand

Der 16-Prozess-Lauf mit dem kleinen Korpus und allen drei Parallelitätsstufen
ist unter Windows-Excel erfolgreich durchgelaufen; alle absichtlich geänderten
Zellen wurden mit PNG-Belegen lokalisiert. Die großen Prioritätsdateien,
insbesondere Kernhaushalte, sind damit noch nicht vermessen. Der JavaScript-
Einzeltest `web/transport.test.cjs` konnte hier mangels Node nicht laufen.

Lokaler Lauf vom 23.10.2026 mit `--ignore-memory-guard`, minimierten Fenstern
und zwei Wiederholungen; alle sechs Läufe waren vollständig:

| Aktive Gruppen | Lauf 1 | Lauf 2 | Mittel | Beschleunigung zum Mittel von 1 |
| --- | ---: | ---: | ---: | ---: |
| 1 | 9,94 s | 9,25 s | 9,60 s | 1,00× |
| 2 | 6,87 s | 6,20 s | 6,54 s | 1,47× |
| 4 | 5,48 s | 6,03 s | 5,75 s | 1,67× |

Vier Gruppen brachten gegenüber zwei im Mittel noch 0,78 s, mit deutlicher
Schwankung zwischen den Wiederholungen. Der kleinste beobachtete freie RAM
lag bei 6,4 GiB. Die aufsummierte Hash-Zeit aller Worker lag je Lauf nur bei
0,12–0,16 s. Diese Zahlen gelten für die kleinen gewählten Capture-Bereiche;
die Standardauswahl mit Kernhaushalte benötigt einen eigenen Lauf.

Ein zusätzlicher Lauf mit der endgültigen eingebauten Fensterminimierung,
ohne externen Helfer, beendete alle drei Modi ebenfalls erfolgreich
(8,16 / 6,91 / 5,56 s). Nach jedem Lauf waren alle Excel- und Host-Prozesse
beendet.
