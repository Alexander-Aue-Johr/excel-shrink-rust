# Excel-Höhenprüfung

VS Code: **Terminal → Run Task → Excel tools: …**. Dateiauswahl: `*` für den ganzen lokalen Korpus oder ein exakter XLSX-Dateiname. Alle Tasks bauen zuerst die aktuelle Release-CLI. Bestehende Tasks bleiben erhalten.

Tasks:

- **Shrink only**: Default oder Maximum Cleaning wählen; kein Excel nötig.
- **Default heights**: aktueller Standardlauf mit Excel-Höhenprüfung.
- **Maximum Cleaning heights**: aggressive Bereinigung prüfen.
- **All profile heights**: alle acht vordefinierten Profile vergleichen.
- **Default heights after Excel save**: nur Originalkopien vor der Messung mit Excel speichern, schließen und neu öffnen. Shrink verwendet weiterhin die unveränderten Originale.

Ergebnisse: `.tmp/height-checks/<Zeitstempel-ID>/`. `manifest.json` hält Profile, Quell-Hashes und Shrink-Zeiten fest; `report.md` die Übersicht; `differences.csv` nur Abweichungen; `comparison.json` alle Blattmessungen. Die bereinigten Dateien und Referenzkopien bleiben für Untersuchungen erhalten. Keine Ergebnisse vorheriger Läufe werden überschrieben. Exitcode 0 = erfolgreich/keine Höhenabweichung, 2 = Höhenabweichung, andere Codes = Fehler. Fehlgeschlagene Messungen erzeugen keinen vollständigen Erfolgsbericht.

## Was gemessen wird

Die VBA-Funktion ermittelt pro Shrunk-Blatt die UsedRange-Adresse. **Genau dieselbe Adresse** wird im Original gemessen. Verglichen wird Range.Height in Punkten, zusätzlich umgerechnet mit ×4/3 auf Pixeläquivalente bei 96 DPI und 100 %. Keine real gemessenen Screenshot-Pixel. Gleiche Gesamthöhe schließt gegenläufige Änderungen einzelner Zeilen nicht aus. Für sichtbare Unterschiede weiterhin die vorhandenen visuellen Regressionstasks verwenden.

Je Workbook/Profile erfolgen zwei VBA-Batches statt COM-Aufrufen pro Blatt. Originaldateien bleiben unverändert; nur Kopien können im optionalen Kontrolllauf gespeichert werden. Excel wird in einer eigenen Instanz gestartet; nur diese Instanz wird geschlossen. Makros der XLSX-Dateien bleiben deaktiviert. Nur der mitgelieferte Helfer wird mit aktivierten Makros geöffnet; AccessVBOM und globale Sicherheitseinstellungen werden nicht geändert.

## Voraussetzungen und Aufruf

WSL mit Rust/Cargo und Python 3; auf Windows installiertes Desktop-Excel sowie Windows PowerShell 5.1. WSL muss `powershell.exe` starten können. Unter Windows: Python 3 und VS Code auf dem UNC-Pfad des WSL-Repositories (`\\wsl.localhost\<Distro>\…`). Native Windows-Rust-Checkouts unterstützt dieser Runner nicht.

```sh
python3 tools/excel-height-check/run.py --profile default --height
python3 tools/excel-height-check/run.py --profile all --height --only-file foerderprogramme-5411206209005.xlsx
python3 tools/excel-height-check/run.py --profile default --height --resave-originals
```

Der Korpus liegt in `corpus-local/original_destatis_xlsx_files`. `HeightProbeHost.xlsm` enthält das getestete Modul `HeightProbe.bas`; beide werden versioniert. Änderungen an der BAS-Datei ändern das eingebettete Makro nicht automatisch: dazu muss der Helfer neu erstellt werden. Der Helfer-Hash wird pro Lauf aufgezeichnet.

Die Profilflags entsprechen `excel-shrink-cli/src/visual_profile.rs`; bei Änderungen dort auch `PROFILES` im Runner aktualisieren. Die bisherigen Screenshot-/C#-Benchmarks bleiben getrennte Experimente. Dieser Runner konzentriert sich auf den reproduzierbaren Shrink- und Höhenvergleich.

Die Profilordner heißen `p00`, `p01` usw., um Windows-Pfadgrenzen zu vermeiden; `manifest.json` ordnet die Namen zu. Excel öffnet kurze lokale Arbeitskopien. Validiert: Windows-Default-Aufruf und WSL-Aufruf aller acht Profile inklusive Excel-Speicherkontrolle auf der einblättrigen Inhaltsübersicht aus dem Korpus.


## Interaktive Datei-/Ordnerauswahl mit VS-Code-JSON-Diff

Task **Excel tools: Select file or folder - Shrink - JSON diff**:

1. Datei oder Ordner wählen (native Windows-Auswahl). Ordner: alle XLSX-Dateien direkt darin, ohne Unterordner und ohne `~$`-Lockdateien.
2. Eines der acht Shrink-Profile wählen.
3. Aktuellen Release-Build erstellen und Kopien der Eingaben bereinigen.
4. Excel/VBA erfasst Metadaten in eigener Excel-Instanz; Originale bleiben unverändert. Kein Excel-Neuspeichern in diesem Task.
5. VS Code öffnet `original.json` links und `shrunk.json` rechts über `code --diff`.

Ergebnisse unter `.tmp/excel-json-diff/<Zeitstempel-ID>/`. Dateien in `input` und `shrunk` verwenden kurze IDs gegen Windows-Pfadgrenzen; `manifest.json` ordnet IDs den Originalnamen und Quellpfaden zu. In beiden Vergleichs-JSONs steht derselbe Original-Dateiname. Laufzeiten, Pfade und Hashes stehen getrennt im Manifest bzw. in `excel-timings.json`, nicht als künstliche Unterschiede im Diff.

Pro Worksheet: Blattindex/-name, Sichtbarkeit (`-1` sichtbar, `0` hidden, `2` very hidden), eigene UsedRange-Adresse, erste/letzte Zeile und Spalte, Zeilen-/Spaltenanzahl, Höhe/Breite in Punkten, Standardhöhe in Punkten und Standardspaltenbreite in Excel-Zeicheneinheiten. Unter `shrunk_range_comparison` stehen zusätzlich Höhe/Breite desselben Bereichs: der Shrunk-UsedRange, auch im Original gemessen. Unterschiedliche eigene UsedRange-Höhen sind allein kein Layoutfehler.

Für automatisierte Aufrufe kann die Auswahl übersprungen werden:

```sh
python3 tools/excel-height-check/json_diff.py --input /pfad/zur/datei.xlsx --profile default
python3 tools/excel-height-check/json_diff.py --input /pfad/zum/ordner --profile maximum-cleaning --no-diff
```

Aus WSL funktionieren auch Windows-Pfade als `--input`. Auf Windows wird `python` verwendet. Zum automatischen Öffnen muss die VS-Code-CLI `code` erreichbar sein; alternativ wird die übliche Windows-Benutzerinstallation erkannt. Fehlt sie, bleiben beide JSON-Dateien erhalten und die Fehlermeldung nennt den Ergebnisordner. Dialogabbruch startet keinen Shrink-Lauf.

Geprüft: Einzeldatei über WSL sowie externer Windows-Ordner mit Leerzeichen/Umlaut, Maximum Cleaning, VBA-Erfassung und erfolgreicher Aufruf des VS-Code-Diff. Der native Picker ist als Windows-Forms-Dialog implementiert; die End-to-End-Tests verwenden die äquivalente `--input`-Schnittstelle.
