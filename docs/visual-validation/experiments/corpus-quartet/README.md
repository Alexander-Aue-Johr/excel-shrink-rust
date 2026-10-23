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

## Tower-Messung vom 23.10.2026

Maschine: **GOTHAM**, AMD Ryzen 7 7800X3D (8 Kerne, 16 logische Prozessoren),
95,06 GiB vom Betriebssystem gemeldeter physischer RAM. Die Messungen verwendeten
minimierte Fenster und `--ignore-memory-guard`. Während der nativen Messungen
liefen keine anderen von uns gestarteten Excel-Tests oder Builds.

Die folgenden Durchsatzwerte gehören zur Capture-Runtime
`af2f8d49c38f13f2f130bcb2b0d2e62a81a3dbb2`, CLI-SHA-256
`5bcc4884ab7f098df00a80f1fd86abba9db9ce7c6b9071397ac929f105d36dc6`
und dem atomischen Scheduler aus `b0bf7fc`. Die Reihenfolge der aktiven Gruppen
war pro Wiederholung verschieden: 1/2/4, 4/2/1, 2/4/1.

### Kleine Auswahl nach dem Signal-Fix

| Aktive Gruppen | Wiederholung 1 | Wiederholung 2 | Wiederholung 3 | Mittel | Verhältnis der Mittelwerte |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 4,985 s | 5,103 s | 4,963 s | 5,017 s | 1,00× |
| 2 | 2,771 s | 2,745 s | 3,225 s | 2,914 s | 1,72× |
| 4 | 2,100 s | 2,114 s | 2,192 s | 2,135 s | 2,35× |

**9/9 Versuche vollständig; 108/108 absichtlich geänderte Profilzellen erkannt.**
Jeder erfolgreiche Versuch musste alle zwölf Kontrolländerungen mit einer
passenden Pixelregion und beiden PNG-Belegen nachweisen. Die visuellen Befunde
mussten zwischen den Wiederholungen übereinstimmen. Der kleinste gemeldete freie
RAM betrug 63,4 GiB.

Die Tabelle misst Capture, Hash, Verfeinerung und PNG-Übertragung bei bereits
geöffneten 16 Excel-Prozessen. Der gesamte Neuner-Durchlauf einschließlich
Staging, Excel-Start und Aufräumen dauerte **326,10 s**. Der Faktor 2,35 beschreibt
deshalb ausschließlich die gemeinsame Capture-Messstrecke. Ein Speedup für die
gesamte Datenpipeline wurde damit nicht gemessen.

Rohwerte: [small-fixed.csv](results/tower-20261023/small-fixed.csv).
Der lokale Lauf samt HTML und Bildbelegen liegt unter
`.tmp/corpus-quartet-tower-small-fixed-20261023`.

### Erster Durchlauf mit zwei Signalfehlern

Die vorherigen Beobachtungen bleiben erhalten:

| Aktive Gruppen | Wiederholung 1 | Wiederholung 2 | Wiederholung 3 | Vollständig | Validierte Kontrollen |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Startfehler | Messabbruch | 4,943 s | 1/3 | 12/36 |
| 2 | 2,756 s | 2,777 s | 2,695 s | 3/3 | 36/36 |
| 4 | 2,141 s | 2,142 s | 2,108 s | 3/3 | 36/36 |

Bei r0/c1 konnte PowerShell `phase-3.launch` nicht lesen, bei r1/c1
`measurement.signal`: Python hatte den endgültigen Dateinamen bereits angelegt,
aber den Writer noch nicht geschlossen. Der Scheduler schreibt jetzt zuerst
eine `.pending`-Datei, schließt sie und veröffentlicht den Token per Rename.
Ein deterministischer Test hält den Writer absichtlich offen und kontrolliert,
dass der endgültige Name währenddessen unsichtbar bleibt. Fehlgeschlagene
Versuche erhalten keinen Speedup-Wert.

Rohwerte: [small-initial.csv](results/tower-20261023/small-initial.csv).
Vollständige lokale Artefakte: `.tmp/corpus-quartet-tower-small-20261023`.

### Große Auswahl: Startgrenze und isolierte Folgeprobe

Der erste Prioritätsversuch erreichte die gemeinsame Startlinie nicht:
Das Kernhaushalte-Original meldete sich innerhalb von 300 s nicht bei Office.js
an. Der Fehler trat bereits beim ersten Original auf; die weiteren Excel-
Instanzen warteten noch auf ihre Startfreigabe. Die erste Fehlbeobachtung ist
gespeichert. Den bereits angelaufenen zweiten Versuch haben wir gezielt beendet,
anstatt weitere identische Timeouts abzuwarten. Es gibt **keinen Capture-
Durchsatzwert für die große Vierer-Auswahl**.

Eine Auffälligkeit war die vorbereitete Blattauswahl: Captureziel war Blatt 84,
`activeTab` zeigte darauf, `tabSelected=1` stand aber weiterhin bei Blatt 1.
Excel zeigte im Fenstertitel „Group“. Die neue Vorbereitung setzt eine
konsistente Auswahl ausschließlich in den Wegwerfkopien. Zellinhalte,
Formatierung und Zeilenhöhen werden dabei erhalten; `firstSheet` bleibt erhalten.

Eine kleine Zwei-Blatt-Gegenprobe ist wichtig: **Sowohl die widersprüchliche
als auch die konsistente Variante meldeten Office.js erfolgreich an**
(2,80 / 3,17 s einschließlich Staging und absichtlichem Abbruch). Das Auswahl-
Muster allein bewies den Timeout damit nicht.

| Weitere Großprobe | Ergebnis | Gemessener Bezug |
| --- | --- | --- |
| Erster Probe-Harness mit zu kleinen Platzhaltern | Vor Excel-Start abgelehnt: Blattindex 84 fehlt in ungeöffneten Platzhaltern | Fixture-Fehler, kein nativer Startbefund |
| Korrigierter Harness, konsistente Auswahl | Original meldet Office.js an; danach absichtlich vor Excel Nr. 2 beendet | 46,40 s für Staging, einen Start und Aufräumen; Startlimit 90 s |

Die erfolgreiche Großprobe verwendete dasselbe Original-SHA-256
`b4a3dd77023756e06c3b7c9b04a55a8d3fbf4334f8803f44460a8dc8cd4dc603`
und die später in `ca143bf53b3f2b018e32ba59fb2a443add533600` gespeicherten
Helfer. Nach Neuordnung der Main-Commits ist derselbe Quellbaum unver?ndert
als `2fccb62c11dbbc97bcf9736dff07cb4a188dd237` erreichbar. Ihre C#-Datei hat SHA-256
`848ffc3141044528425cba101446b854542a4459048635710e5174f9b1b06524`.
Die PS-Helferdatei hat SHA-256
`24516c5096e8a181c842ab98bf369a2db52e86c8dcef170f5baff12528aea669`.

Diese Ein-Excel-Probe hat andere Staging-Eingaben und keine gemeinsame
16-Prozess-Startlinie. Ihre 46,40 s sind weder mit der Capture-Tabelle noch
als gepaarter Startup-Speedup gegenüber dem 300-s-Timeout vergleichbar.
Die erfolgreiche Anmeldung stützt die Normalisierung als sinnvolle Korrektur;
sie belegt keine Heilung aller Add-in-Startfehler. Einen neuen großen Neuner-
Durchlauf haben wir zugunsten der produktiven Regression ausgesetzt.

Rohwert des ursprünglichen Fehlversuchs:
[priority-initial.csv](results/tower-20261023/priority-initial.csv).
Die [Beobachtungen mit Pins, Eingabe- und Artefaktprüfsummen](results/tower-20261023/observations.json)
enthalten auch beide kleinen Gegenproben, den Fixture-Fehler, die erfolgreiche
Großprobe und die Pfade zu den unveränderten lokalen Rohdaten.

Für weitere reproduzierbare Versuche ist die Runtime nun auf `2fccb62` gepinnt.
Das Quartett bereitet Auswahl und Fehlerindikatoren damit gemeinsam in einem
Streamingpass vor. `sheet_selection_probe.py` kann mit `--runtime` entweder
die ursprüngliche oder die korrigierte exportierte Runtime prüfen; für eine
reale Mappe werden zusätzlich `--workbook` und `--scope` angegeben. Es öffnet
höchstens eine Excelinstanz und beendet sie nach der Office.js-Anmeldung,
bevor die zweite Instanz startet.

Die kleine Gegenprobe lässt sich ohne vorherigen Vierer-Benchmark exportieren
und starten:

```powershell
python docs/visual-validation/experiments/corpus-quartet/sheet_selection_probe.py --output .tmp/selection-probe --runtime-ref af2f8d49c38f13f2f130bcb2b0d2e62a81a3dbb2
```

Ohne `--runtime-ref` gilt der aktuelle geprüfte Pin. Die reale Großprobe wird
mit neuem Ausgabeordner und zusätzlich diesen Argumenten gewählt:
`--startup-timeout 90 --workbook corpus-local/original_destatis_xlsx_files/statistischer-bericht-rechnungsergebnis-kernhaushalt-gemeinden-2140331217005.xlsx --scope "csv-71717-25!A1:G400"`.
