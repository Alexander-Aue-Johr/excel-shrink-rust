<!-- Managed by patch_excel_json_diff.py. -->
# Excel JSON diff: Rust + PowerShell + VBA

Der Task **Excel tools: Select file or folder - Shrink - JSON diff** startet jetzt
`json_diff.ps1`, unter WSL ueber den kleinen Launcher `json_diff_wsl.sh`.
Python ist nur fuer das einmalige Einspielen des Patches erforderlich. Die alten
Python-Runner und die Regressionstests bleiben unveraendert.

## Ablauf

Datei/Ordner und Profil waehlen -> Excel/VBA-Vorpruefung -> unveraenderte Eingabekopien
-> vorhandene Rust-CLI ueber `cargo run --release --locked` -> Excel-Read-only-Kopien
-> VBA-UsedRange-Metadaten und echte Zeilenmessungen -> validierte JSON-Dateien
-> VS-Code-Diff. Die Profile werden beim Einspielen aus den tatsaechlichen
`PREDEFINED_VISUAL_PROFILES` in `visual_profile.rs` gewonnen. Aendern sich diese
Definitionen, den Python-Patcher erneut ausfuehren, um den Snapshot zu aktualisieren.

Ordner werden wie bisher **nicht rekursiv** bearbeitet. `.xlsx` und `.XLSX` werden
erkannt; `~$`-Lockdateien werden ausgeschlossen. Kurze eindeutige IDs verhindern
Dateinamen-/Pfadprobleme in den Arbeitskopien. Manifest und Vergleichsansicht
behalten die Originalnamen.

## Aufruf unter WSL

```bash
bash tools/excel-height-check/json_diff_wsl.sh
bash tools/excel-height-check/json_diff_wsl.sh -InputPath '/pfad/Datei.xlsx' -Profile default
bash tools/excel-height-check/json_diff_wsl.sh -InputPath 'C:\Daten\Datei.xlsx' -NoDiff
```

Windows PowerShell:

```powershell
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File .\tools\excel-height-check\json_diff.ps1
```

Der Windows-Aufruf erkennt ein Repository auf `\\wsl.localhost\Distro\...` oder
`\\wsl$\Distro\...` und startet Rust in genau dieser Distribution. Fuer einen nativen
Windows-Checkout muss Windows-Rust/Cargo installiert sein. WSL muss den Zugriff
auf Windows-Dateien/Interop erlauben. Der WSL-Launcher startet `code --diff` in der
urspruenglichen WSL-Sitzung, damit deren Remote-VS-Code-Umgebung erhalten bleibt.

## VBA-Helfer: kein veraltetes eingebettetes Makro

`JsonDiffProbe.bas` ist ein **neues, getrenntes** Modul. Die bestehende
`HeightProbeHost.xlsm` wird weder veraendert noch verwendet.

Die Vorpruefung erstellt bei Bedarf einen eigenen kleinen XLSM-Helfer aus dem
aktuellen BAS-Modul. Dieser Vorgang braucht Zugang zum VBA-Projektobjektmodell.
Das Programm aktiviert diese Berechtigung **nicht selbst**, aendert keine
Registry-/Trust-Center-Einstellung und versucht keinen Fallback mit anderen
Messverfahren. Ist der Zugang gesperrt, erscheint vor dem Shrink eine klare
Fehlermeldung mit den folgenden Optionen:

* Ohne VBA-Projektzugriff fuer Automatisierung: eine leere Excel-Arbeitsmappe
  anlegen, mit Alt+F11 den VBA-Editor oeffnen, das gepruefte `JsonDiffProbe.bas`
  importieren und als `tools/excel-height-check/JsonDiffProbeHost.xlsm` speichern.
  Anschliessend den Task erneut starten. Alternativer Ort: `-HelperWorkbook PFAD`.
* Oder den Zugriff zum **Erstellen des Helfers** bewusst in Excel unter
  Trust Center -> Makroeinstellungen -> Zugriff auf das VBA-Projektobjektmodell
  vertrauen zulassen und danach wieder deaktivieren. Der aus dem aktuellen
  BAS-Hash abgeleitete Helfer-Cache kann danach ohne diesen Zugriff laufen.

Der Helfer muss die erwartete Versionskennung liefern. Die Vorpruefung fuehrt
zusaetzlich einen VBA-Selbsttest mit unterschiedlichen Zeilenhoehen, versteckter
Zeile und Batch-Start ungleich 1 aus. Das Test-Workbook ist nur eine Wegwerfkopie.
Geaenderter VBA-Quelltext erhaelt einen eigenen Cache-Pfad; manuell importierte
Helfer muessen bei einer neuen Modulversion ebenfalls aktualisiert werden.

Nur der bekannte/eigens bereitgestellte Helfer wird mit aktivierten Makros
geoeffnet. Eingabekopien werden mit deaktivierten Makros, deaktivierten Events,
ohne Link-Updates und schreibgeschuetzt geoeffnet. Sie werden **nicht gespeichert,
nicht per AutoFit angepasst und nicht explizit neu berechnet**. Gemessen wird
mit manueller Berechnung wie beim bisherigen Task. Excel-eigene Effekte beim
Oeffnen werden damit nicht pauschal ausgeschlossen.

## Messumfang und JSON

Standard: **alle Zeilen aller Worksheets**, einschliesslich der Zeilen ausserhalb
des UsedRange. In einer regulaeren XLSX-Tabelle sind das 1.048.576 Zeilen je Blatt.
`-BatchRows 8192` bestimmt nur die Groesse eines VBA-Aufrufs, nicht den Messumfang.
VBA liest jede Zeile ueber `Rows(r).Height` (effektive Hoehe in Punkten) und
`Rows(r).Hidden`. Fuer versteckte Zeilen ist die effektive Hoehe normalerweise 0;
die gespeicherte Hoehe nach einem spaeteren Einblenden wird hier nicht behauptet.

Gleiche aufeinanderfolgende Werte werden **verlustfrei als Intervalle** gespeichert:

```json
"row_heights": [
  {"from":1,"to":18,"height_points":15,"hidden":false},
  {"from":19,"to":19,"height_points":30,"hidden":false},
  {"from":20,"to":1048576,"height_points":15,"hidden":false}
]
```

Die Intervalle sind inklusive. PowerShell vereinigt gleiche Intervalle ueber
Batch-Grenzen hinweg und prueft lueckenlose Abdeckung, Reihenfolge, Werte und
Vollstaendigkeit. TSV-Zwischenwerte und das endgueltige JSON werden gestreamt;
PowerShell baut keine Liste mit einer Million Zeilenobjekten auf.

Die eigene UsedRange wird vor dem Zeilenscan ermittelt. Zusaetzlich wird die
Shrunk-UsedRange-Adresse auch im Original vermessen. Verschiedene eigene
UsedRanges bzw. ein nicht leeres JSON-Diff sind daher nicht automatisch ein
Layoutfehler. Vergleiche hierzu die Zeilenhoehen und `shrunk_range_comparison`.

Explizit begrenzter Diagnosemodus:

```bash
bash tools/excel-height-check/json_diff_wsl.sh -RowScope used-union
```

Dieser Modus erfasst Zeile 1 bis zur groesseren letzten UsedRange-Zeile beider
Dateien. Er ist **kein vollstaendiger Arbeitsblattvergleich**. Der abweichende
Umfang wird sichtbar in `row_scan.scope` und im Log vermerkt.

## Logs, Fehler und Abbruch

Ergebnisse: `.tmp/excel-json-diff/<Zeitstempel-ID>/`

* `runner.log`: laufende Ausgabe von PowerShell, Cargo/Rust und Excel-Worker;
  native stdout/stderr werden gleichzeitig gelesen und unmittelbar ausgegeben.
* `manifest.json`, `probe-info.json`, `excel-timings.json`: Quellen, Hashes,
  Profilflags, VBA-/Excel-Version und Laufzeiten, getrennt vom eigentlichen Diff.
* `original.json` / `shrunk.json`: Vergleichsdateien; `.partial` bedeutet noch
  nicht erfolgreich abgeschlossen.
* `rows/`: rohe vollstaendige TSV-Intervalle zur Diagnose; `*.raw.json`:
  Zwischenmetadaten. Die Daten bleiben auch bei Fehlern zur Untersuchung erhalten.

Bei laufenden Kindprozessen gibt es alle fuenf Sekunden ein Lebenszeichen.
`-StallSeconds 1800` begrenzt die Zeit **ohne echte Ausgabe des Kindes**; die
Lebenszeichen des Supervisors setzen diesen Timer nicht zurueck. Vor/nach dem
Oeffnen eines Workbooks und bei jedem VBA-Batch gibt es eigene Meldungen.
Ein Rust-Exitcode 0 allein reicht nicht: jede erwartete Shrunk-Datei muss existieren
und nicht leer sein. Das beweist noch nicht die semantische Korrektheit des Shrink;
dafuer bleiben die vorhandenen Regressionstests zustaendig.

Der Worker ermittelt den PID seiner eigenen neu erzeugten Excel-Instanz.
Aufraeumen/Timeout verwenden zusaetzlich deren Prozessstartzeit und ein Lauf-Token;
es gibt kein `taskkill /IM EXCEL.EXE` und kein Beenden aller Excel-Prozesse.
Vorhandene Benutzerinstanzen werden nicht absichtlich geschlossen. Wird ein
bereits vorhandener Excel-Prozess wiederverwendet, bricht die Vorpruefung ab.
Bei hartem Beenden des gesamten Prozessbaums oder COM-Aktivierungsfehlern vor der
PID-Ermittlung kann kein vollstaendiges Aufraeumen garantiert werden.

## Tests

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\excel-height-check\test_json_diff_native.ps1
```

Diese Tests benoetigen kein Excel: PowerShell-Syntax, Argument-Quoting,
Intervallvalidierung/-vereinigung, JSON-Struktur, stdout/stderr-Streaming,
Fehlercode und Stall-Abbruch. Den echten Excel-/COM-/WSL-End-to-End-Test fuehrt
anschliessend der Task auf dem Windows-Rechner durch. Der Patch installiert keine
zusaetzlichen Pakete und fuehrt beim Einspielen selbst noch kein Workbook aus.
