// .NET Framework 4.5 / C# 5 compatible. Loaded by Windows PowerShell 5.1.
// Read-only OOXML planning: no Excel interop, no workbook writes, no external I/O.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Xml;
using System.Xml.Linq;

namespace ExcelShrink.Visual {
    public sealed class TextCandidate {
        public int Row, Column, MergeEndRow, MergeEndColumn;
        public string Text, StyleKey;
        public bool RichText, NeedsLiveFormatting;
    }
    public sealed class PointAnchor {
        public string Name;
        // A zero row/column denotes an absolute point position.
        public int Row, Column;
        public double X, Y, Width, Height, Padding;
    }
    public sealed class WorksheetPlan {
        public readonly List<TextCandidate> Text = new List<TextCandidate>();
        public readonly List<PointAnchor> PointAnchors = new List<PointAnchor>();
        public readonly List<string> FallbackShapes = new List<string>();
        public readonly List<string> AmbiguousFallbackShapes = new List<string>();
        public bool AllShapesNeedCom;
        public int ShapeCount, FastTwoCellShapes, MaxMarkerRow, MaxMarkerColumn;
        public double MaxRowOffset, MaxColumnOffset, Padding;
        public int ScannedRows, ScannedCells;
        public bool HasConditionalFormatting;
    }
    internal sealed class ColumnStyle {
        public int First, Last, Style;
        public bool Hidden;
    }
    internal sealed class Merge {
        public int Top, Left, Bottom, Right;
    }
    internal sealed class SharedText {
        public string Text;
        public bool Rich;
    }
    internal sealed class Relationship {
        public string Id, Type, Target;
        public bool External;
    }
    public sealed class WorkbookPlan : IDisposable {
        private readonly FileStream stream;
        private readonly ZipArchive zip;
        private readonly Dictionary<string, string> worksheets = new Dictionary<string, string>(StringComparer.Ordinal);
        private readonly Dictionary<string, WorksheetPlan> plans = new Dictionary<string, WorksheetPlan>(StringComparer.Ordinal);
        private readonly List<int> numberFormats = new List<int>();
        private List<SharedText> sharedStrings;
        private string stringsPart;

        public WorkbookPlan(string path) {
            stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            try {
                zip = new ZipArchive(stream, ZipArchiveMode.Read, false);
                var rels = Relationships("xl/workbook.xml");
                var book = Load("xl/workbook.xml");
                foreach (var sheet in book.Descendants().Where(e => e.Name.LocalName == "sheet")) {
                    string id = AttrLocal(sheet, "id");
                    Relationship rel;
                    if (id != null && rels.TryGetValue(id, out rel) && !rel.External && rel.Type.EndsWith("/worksheet", StringComparison.Ordinal)) {
                        worksheets.Add(Required(sheet, "name"), Resolve("xl/workbook.xml", rel.Target));
                    }
                }
                foreach (var rel in rels.Values) {
                    if (rel.External) continue;
                    if (rel.Type.EndsWith("/sharedStrings", StringComparison.Ordinal)) stringsPart = Resolve("xl/workbook.xml", rel.Target);
                    if (rel.Type.EndsWith("/styles", StringComparison.Ordinal)) {
                        var styles = Load(Resolve("xl/workbook.xml", rel.Target));
                        var xfs = styles.Root.Elements().FirstOrDefault(e => e.Name.LocalName == "cellXfs");
                        var bases = styles.Root.Elements().FirstOrDefault(e => e.Name.LocalName == "cellStyleXfs");
                        var baseFormats = bases == null ? new List<int>() : bases.Elements().Select(e => IntAttr(e, "numFmtId", 0)).ToList();
                        if (xfs != null) foreach (var xf in xfs.Elements()) {
                            int baseIndex = IntAttr(xf, "xfId", 0);
                            int inherited = baseIndex >= 0 && baseIndex < baseFormats.Count ? baseFormats[baseIndex] : 0;
                            numberFormats.Add(IntAttr(xf, "numFmtId", inherited));
                        }
                    }
                }
            } catch { if (zip != null) zip.Dispose(); stream.Dispose(); throw; }
        }
        public void Dispose() { zip.Dispose(); stream.Dispose(); }
        private static XmlReaderSettings Settings() {
            return new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null, IgnoreComments = true, IgnoreProcessingInstructions = true };
        }
        private ZipArchiveEntry Entry(string part) {
            var entry = zip.GetEntry(part);
            if (entry == null) throw new InvalidDataException("Missing OOXML part: " + part);
            return entry;
        }
        private XDocument Load(string part) {
            using (var input = Entry(part).Open())
            using (var reader = XmlReader.Create(input, Settings())) return XDocument.Load(reader);
        }
        private static string RelPart(string part) {
            int slash = part.LastIndexOf('/');
            return slash < 0 ? "_rels/" + part + ".rels" : part.Substring(0, slash + 1) + "_rels/" + part.Substring(slash + 1) + ".rels";
        }
        private Dictionary<string, Relationship> Relationships(string part) {
            var result = new Dictionary<string, Relationship>(StringComparer.Ordinal);
            var entry = zip.GetEntry(RelPart(part));
            if (entry == null) return result;
            using (var input = entry.Open())
            using (var reader = XmlReader.Create(input, Settings())) {
                var document = XDocument.Load(reader);
                foreach (var node in document.Root.Elements().Where(e => e.Name.LocalName == "Relationship")) {
                    var rel = new Relationship { Id = Required(node, "Id"), Type = Required(node, "Type"), Target = Required(node, "Target"), External = (string)node.Attribute("TargetMode") == "External" };
                    result.Add(rel.Id, rel);
                }
            }
            return result;
        }
        private static string Resolve(string source, string target) {
            if (String.IsNullOrEmpty(target) || target.IndexOfAny(new char[] { '\\', '?', '#', ':' }) >= 0) throw new InvalidDataException("Unsupported OPC target: " + target);
            var parts = new List<string>();
            if (!target.StartsWith("/", StringComparison.Ordinal)) {
                int slash = source.LastIndexOf('/');
                if (slash >= 0) parts.AddRange(source.Substring(0, slash).Split('/'));
            }
            foreach (var component in Uri.UnescapeDataString(target).Split('/')) {
                if (component == "" || component == ".") continue;
                if (component == "..") {
                    if (parts.Count == 0) throw new InvalidDataException("OPC target escapes package");
                    parts.RemoveAt(parts.Count - 1);
                } else parts.Add(component);
            }
            return String.Join("/", parts.ToArray());
        }
        private static string Required(XElement element, string name) {
            var attr = element.Attribute(name);
            if (attr == null) throw new InvalidDataException("Missing " + name + " in " + element.Name);
            return attr.Value;
        }
        private static string AttrLocal(XElement element, string name) {
            var attr = element.Attributes().FirstOrDefault(a => a.Name.LocalName == name);
            return attr == null ? null : attr.Value;
        }
        private static int IntAttr(XElement element, string name, int fallback) {
            var attribute = element.Attribute(name);
            return attribute == null ? fallback : Int(attribute.Value);
        }
        private static int Int(string value) { return Int32.Parse(value, NumberStyles.Integer, CultureInfo.InvariantCulture); }
        private static double Number(string value) {
            double result = Double.Parse(value, NumberStyles.Float, CultureInfo.InvariantCulture);
            if (Double.IsNaN(result) || Double.IsInfinity(result)) throw new InvalidDataException("Non-finite drawing coordinate");
            return result;
        }
        private static bool True(string value) { return value == "1" || String.Equals(value, "true", StringComparison.OrdinalIgnoreCase); }
        private static SharedText ReadText(XElement element) {
            // Exclude phonetic rPh runs. They are annotations, not the base cell text.
            string text = String.Concat(element.Descendants().Where(e => e.Name.LocalName == "t" && !e.Ancestors().Any(a => a.Name.LocalName == "rPh")).Select(e => e.Value));
            return new SharedText { Text = DecodeExcelEscapes(text), Rich = element.Descendants().Any(e => e.Name.LocalName == "r") };
        }
        private static string DecodeExcelEscapes(string text) {
            // Single left-to-right pass: _x005F_x0041_ means literal _x0041_, not A.
            var result = new System.Text.StringBuilder(text.Length);
            for (int i = 0; i < text.Length; i++) {
                ushort code;
                if (i + 6 < text.Length && text[i] == '_' && text[i + 1] == 'x' && text[i + 6] == '_'
                    && UInt16.TryParse(text.Substring(i + 2, 4), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out code)) {
                    result.Append((char)code); i += 6;
                } else result.Append(text[i]);
            }
            return result.ToString();
        }
        private void EnsureStrings(Action<string> progress) {
            if (sharedStrings != null) return;
            var result = new List<SharedText>();
            if (stringsPart != null) {
                progress("bounds OOXML shared strings start");
                using (var input = Entry(stringsPart).Open())
                using (var reader = XmlReader.Create(input, Settings())) {
                    var timer = Stopwatch.StartNew();
                    while (reader.Read()) {
                        if (reader.NodeType != XmlNodeType.Element || reader.LocalName != "si") continue;
                        using (var subtree = reader.ReadSubtree()) result.Add(ReadText(XElement.Load(subtree)));
                        if (timer.Elapsed.TotalSeconds >= 2) { progress("bounds OOXML shared strings count=" + result.Count); timer.Restart(); }
                    }
                }
                progress("bounds OOXML shared strings done count=" + result.Count);
            }
            sharedStrings = result;
        }
        private static void CellAddress(string address, out int row, out int column) {
            column = 0; int i = 0;
            if (String.IsNullOrEmpty(address)) throw new InvalidDataException("Missing cell address");
            while (i < address.Length && (address[i] == '$' || Char.IsLetter(address[i]))) {
                if (address[i] != '$') column = checked(column * 26 + Char.ToUpperInvariant(address[i]) - 'A' + 1);
                i++;
            }
            if (i < address.Length && address[i] == '$') i++;
            row = Int(address.Substring(i));
            if (row < 1 || row > 1048576 || column < 1 || column > 16384) throw new InvalidDataException("Invalid cell address " + address);
        }
        public WorksheetPlan Get(string name, int maxRow, int maxColumn, Action<string> progress) {
            WorksheetPlan cached;
            string planKey = name + "|" + maxRow + "|" + maxColumn;
            if (plans.TryGetValue(planKey, out cached)) return cached;
            string part;
            if (!worksheets.TryGetValue(name, out part)) throw new InvalidDataException("Worksheet not found in OOXML: " + name);
            var plan = new WorksheetPlan();
            var columns = new List<ColumnStyle>(); var merges = new List<Merge>(); var drawingIds = new List<string>();
            var lastByRow = new Dictionary<int, TextCandidate>();
            var lastColumnByRow = new Dictionary<int, int>();
            int currentRow = 0, currentColumn = 0, rowStyle = -1; bool rowHidden = false;
            progress("bounds OOXML scan start worksheet='" + name + "'");
            var timer = Stopwatch.StartNew();
            using (var input = Entry(part).Open())
            using (var reader = XmlReader.Create(input, Settings())) {
                while (reader.Read()) {
                    if (reader.NodeType != XmlNodeType.Element) continue;
                    switch (reader.LocalName) {
                        case "row":
                            currentRow = reader.GetAttribute("r") == null ? currentRow + 1 : Int(reader.GetAttribute("r"));
                            currentColumn = 0; rowHidden = True(reader.GetAttribute("hidden"));
                            rowStyle = True(reader.GetAttribute("customFormat")) && reader.GetAttribute("s") != null ? Int(reader.GetAttribute("s")) : -1;
                            plan.ScannedRows++;
                            break;
                        case "col":
                            columns.Add(new ColumnStyle { First = Int(reader.GetAttribute("min")), Last = Int(reader.GetAttribute("max")),
                                Hidden = True(reader.GetAttribute("hidden")), Style = reader.GetAttribute("style") == null ? -1 : Int(reader.GetAttribute("style")) });
                            break;
                        case "c":
                            plan.ScannedCells++;
                            int cellRow = currentRow, column = currentColumn + 1;
                            string address = reader.GetAttribute("r");
                            if (address != null) CellAddress(address, out cellRow, out column);
                            currentColumn = column;
                            if (reader.IsEmptyElement || rowHidden || cellRow > maxRow || column > maxColumn) break;
                            ColumnStyle colStyle = columns.LastOrDefault(c => c.First <= column && column <= c.Last);
                            if (colStyle != null && colStyle.Hidden) break;
                            string cellStyleRaw = reader.GetAttribute("s");
                            int cellStyle = cellStyleRaw == null ? -1 : Int(cellStyleRaw);
                            int effective = cellStyle >= 0 ? cellStyle : rowStyle >= 0 ? rowStyle : colStyle != null && colStyle.Style >= 0 ? colStyle.Style : 0;
                            string type = reader.GetAttribute("t") ?? "n";
                            XElement cell;
                            using (var subtree = reader.ReadSubtree()) cell = XElement.Load(subtree);
                            bool formula = cell.Elements().Any(e => e.Name.LocalName == "f");
                            var v = cell.Elements().FirstOrDefault(e => e.Name.LocalName == "v");
                            SharedText text = null;
                            if (type == "s" && v != null) {
                                EnsureStrings(progress); int stringIndex = Int(v.Value);
                                if (stringIndex < 0 || stringIndex >= sharedStrings.Count) throw new InvalidDataException("Invalid shared-string index");
                                text = sharedStrings[stringIndex];
                            } else if (type == "inlineStr") text = ReadText(cell);
                            else if (type == "str" && v != null) text = new SharedText { Text = DecodeExcelEscapes(v.Value), Rich = false };
                            bool occupied = formula || (text != null && text.Text.Length != 0) || (v != null && v.Value.Length != 0);
                            if (!occupied) break;
                            int previous;
                            if (lastColumnByRow.TryGetValue(cellRow, out previous) && previous > column) break;
                            lastColumnByRow[cellRow] = column;
                            if (text == null || text.Text.Length == 0) { lastByRow.Remove(cellRow); break; }
                            int format = effective >= 0 && effective < numberFormats.Count ? numberFormats[effective] : 0;
                            lastByRow[cellRow] = new TextCandidate {
                                Row = cellRow, Column = column, Text = text.Text, RichText = text.Rich,
                                StyleKey = cellStyle + ":" + rowStyle + ":" + (colStyle == null ? -1 : colStyle.Style),
                                NeedsLiveFormatting = formula || (format != 0 && format != 49),
                                MergeEndRow = cellRow, MergeEndColumn = column,
                            };
                            break;
                        case "mergeCell":
                            string[] endpoints = (reader.GetAttribute("ref") ?? "").Split(':');
                            var merge = new Merge();
                            CellAddress(endpoints[0], out merge.Top, out merge.Left);
                            CellAddress(endpoints[endpoints.Length - 1], out merge.Bottom, out merge.Right);
                            merges.Add(merge); break;
                        case "conditionalFormatting": case "extLst":
                            plan.HasConditionalFormatting = true; break;
                        case "drawing":
                            string rid = reader.GetAttribute("id", "http://schemas.openxmlformats.org/officeDocument/2006/relationships")
                                ?? reader.GetAttribute("id", "http://purl.oclc.org/ooxml/officeDocument/relationships");
                            if (rid == null) plan.AllShapesNeedCom = true; else drawingIds.Add(rid);
                            break;
                        case "legacyDrawing": case "legacyDrawingHF": case "oleObjects": case "controls":
                            plan.AllShapesNeedCom = true; break;
                    }
                    if (timer.Elapsed.TotalSeconds >= 2) {
                        progress("bounds OOXML scan worksheet='" + name + "' cells=" + plan.ScannedCells + " rows=" + plan.ScannedRows);
                        timer.Restart();
                    }
                }
            }
            foreach (var candidate in lastByRow.OrderBy(kv => kv.Key).Select(kv => kv.Value)) {
                candidate.NeedsLiveFormatting |= plan.HasConditionalFormatting;
                foreach (var merge in merges) if (merge.Top <= candidate.Row && candidate.Row <= merge.Bottom && merge.Left <= candidate.Column && candidate.Column <= merge.Right) {
                    candidate.MergeEndRow = merge.Bottom; candidate.MergeEndColumn = merge.Right;
                    candidate.NeedsLiveFormatting = true;
                }
                plan.Text.Add(candidate);
            }
            var rels = Relationships(part);
            foreach (string id in drawingIds.Distinct()) {
                Relationship rel;
                if (!rels.TryGetValue(id, out rel) || rel.External || !rel.Type.EndsWith("/drawing", StringComparison.Ordinal)) {
                    plan.AllShapesNeedCom = true; continue;
                }
                string drawingPart = Resolve(part, rel.Target);
                try { AddDrawing(plan, Load(drawingPart)); }
                catch (Exception error) { plan.AllShapesNeedCom = true; progress("bounds OOXML drawing fallback part='" + drawingPart + "' error='" + error.Message + "'"); }
            }
            progress("bounds OOXML scan done worksheet='" + name + "' candidates=" + plan.Text.Count + " shapes=" + plan.ShapeCount + " fast_two_cell=" + plan.FastTwoCellShapes + " fallback_all=" + plan.AllShapesNeedCom);
            plans.Add(planKey, plan); return plan;
        }
        private static XElement Child(XElement element, string name) { return element.Elements().FirstOrDefault(e => e.Name.LocalName == name); }
        private static double ChildNumber(XElement element, string name) {
            var child = Child(element, name);
            if (child == null) throw new InvalidDataException("Missing anchor " + name);
            return Number(child.Value);
        }
        private static void Marker(XElement marker, out int row, out int col, out double rowOff, out double colOff) {
            if (marker == null) throw new InvalidDataException("Missing anchor marker");
            double rawRow = ChildNumber(marker, "row"), rawCol = ChildNumber(marker, "col");
            if (rawRow != Math.Truncate(rawRow) || rawCol != Math.Truncate(rawCol)) throw new InvalidDataException("Fractional drawing marker");
            row = checked((int)rawRow + 1); col = checked((int)rawCol + 1);
            rowOff = ChildNumber(marker, "rowOff") / 12700.0; colOff = ChildNumber(marker, "colOff") / 12700.0;
            if (row < 1 || row > 1048576 || col < 1 || col > 16384 || rowOff < 0 || colOff < 0) throw new InvalidDataException("Anchor outside worksheet");
        }
        private static readonly HashSet<string> BoundsNeutralDrawingExtensionUris =
            new HashSet<string>(StringComparer.OrdinalIgnoreCase) {
                "{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}", // a16:creationId
                "{909E8E84-426E-40DD-AFC4-6F175D3DCCD1}", // a14:hiddenFill
                "{91240B29-F687-4F45-9708-019B960494DF}", // a14:hiddenLine
            };

        private static bool HasUnsupportedDrawingExtension(XElement anchor) {
            foreach (var list in anchor.Descendants().Where(e => e.Name.LocalName == "extLst")) {
                foreach (var child in list.Elements()) {
                    if (child.Name.LocalName != "ext") return true;
                    string uri = (string)child.Attribute("uri");
                    if (String.IsNullOrWhiteSpace(uri) || !BoundsNeutralDrawingExtensionUris.Contains(uri)) return true;
                }
            }
            return false;
        }

        private static void AddFallbackShape(WorksheetPlan plan, string name, IDictionary<string, int> shapeNameCounts) {
            if (String.IsNullOrEmpty(name)) { plan.AllShapesNeedCom = true; return; }
            plan.FallbackShapes.Add(name);
            int count;
            if (shapeNameCounts.TryGetValue(name, out count) && count > 1) plan.AmbiguousFallbackShapes.Add(name);
        }

        private static void AddDrawing(WorksheetPlan plan, XDocument drawing) {
            var anchors = drawing.Root.Elements().ToList();
            var shapeNameCounts = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (var anchor in anchors) {
                var property = anchor.Descendants().FirstOrDefault(e => e.Name.LocalName == "cNvPr");
                string name = property == null ? null : (string)property.Attribute("name");
                if (String.IsNullOrEmpty(name)) continue;
                int count; shapeNameCounts.TryGetValue(name, out count); shapeNameCounts[name] = count + 1;
            }
            foreach (var anchor in anchors) {
                string kind = anchor.Name.LocalName;
                if (kind != "twoCellAnchor" && kind != "oneCellAnchor" && kind != "absoluteAnchor") { plan.AllShapesNeedCom = true; continue; }
                plan.ShapeCount++;
                var property = anchor.Descendants().FirstOrDefault(e => e.Name.LocalName == "cNvPr");
                string name = property == null ? null : (string)property.Attribute("name");
                if (property != null && True((string)property.Attribute("hidden"))) continue;
                string editAs = (string)anchor.Attribute("editAs");
                bool special = anchor.Descendants().Any(e => e.Name.LocalName == "grpSp" || (e.Name.LocalName == "txBody" &&
                    e.Descendants().Any(t => t.Name.LocalName == "fld" || (t.Name.LocalName == "t" && !String.IsNullOrWhiteSpace(t.Value))))
                    || e.Name.LocalName == "AlternateContent"
                    || ((e.Name.LocalName == "effectLst" || e.Name.LocalName == "effectDag") && e.HasElements)
                    || (e.Name.LocalName == "xfrm" && e.Attribute("rot") != null && Number(e.Attribute("rot").Value) % 21600000.0 != 0))
                    || HasUnsupportedDrawingExtension(anchor);
                if (special) { AddFallbackShape(plan, name, shapeNameCounts); continue; }
                try {
                    double padding = 8;
                    foreach (var line in anchor.Descendants().Where(e => e.Name.LocalName == "ln" && e.Attribute("w") != null)) padding = Math.Max(padding, Number(line.Attribute("w").Value) / 12700.0 * 5);
                    if (kind == "twoCellAnchor" && editAs != null && editAs != "twoCell") {
                        // Fixed-size two-cell anchors use their saved transform extent.
                        // Do not reinterpret their second marker as a resizable shape.
                        if (editAs != "absolute" && editAs != "oneCell") throw new InvalidDataException("Unknown anchor editing behavior");
                        var transform = anchor.Descendants().FirstOrDefault(e => e.Name.LocalName == "xfrm");
                        if (transform == null) throw new InvalidDataException("Fixed anchor has no transform");
                        var extent = Child(transform, "ext");
                        if (extent == null) throw new InvalidDataException("Fixed anchor has no extent");
                        var point = new PointAnchor { Name = name, Padding = padding,
                            Width = Number(Required(extent, "cx")) / 12700.0, Height = Number(Required(extent, "cy")) / 12700.0 };
                        if (point.Width < 0 || point.Height < 0) throw new InvalidDataException("Negative fixed-anchor extent");
                        if (editAs == "absolute") {
                            var position = Child(transform, "off");
                            if (position == null) throw new InvalidDataException("Fixed anchor has no position");
                            point.X = Number(Required(position, "x")) / 12700.0; point.Y = Number(Required(position, "y")) / 12700.0;
                        } else {
                            int r, c; double ro, co; Marker(Child(anchor, "from"), out r, out c, out ro, out co);
                            point.Row = r; point.Column = c; point.X = co; point.Y = ro;
                        }
                        plan.PointAnchors.Add(point);
                    } else if (kind == "twoCellAnchor") {
                        int r, c; double ro, co;
                        foreach (string end in new string[] { "from", "to" }) {
                            Marker(Child(anchor, end), out r, out c, out ro, out co);
                            plan.MaxMarkerRow = Math.Max(plan.MaxMarkerRow, r); plan.MaxMarkerColumn = Math.Max(plan.MaxMarkerColumn, c);
                            plan.MaxRowOffset = Math.Max(plan.MaxRowOffset, ro); plan.MaxColumnOffset = Math.Max(plan.MaxColumnOffset, co);
                        }
                        plan.Padding = Math.Max(plan.Padding, padding); plan.FastTwoCellShapes++;
                    } else {
                        var point = new PointAnchor { Name = name, Padding = padding };
                        var extent = Child(anchor, "ext");
                        if (extent == null) throw new InvalidDataException("Missing drawing extent");
                        point.Width = Number(Required(extent, "cx")) / 12700.0; point.Height = Number(Required(extent, "cy")) / 12700.0;
                        if (point.Width < 0 || point.Height < 0) throw new InvalidDataException("Negative drawing extent");
                        if (kind == "oneCellAnchor") {
                            int r, c; double ro, co; Marker(Child(anchor, "from"), out r, out c, out ro, out co);
                            point.Row = r; point.Column = c; point.X = co; point.Y = ro;
                        } else {
                            var position = Child(anchor, "pos");
                            if (position == null) throw new InvalidDataException("Missing absolute drawing position");
                            point.X = Number(Required(position, "x")) / 12700.0; point.Y = Number(Required(position, "y")) / 12700.0;
                        }
                        plan.PointAnchors.Add(point);
                    }
                } catch { AddFallbackShape(plan, name, shapeNameCounts); }
            }
        }
    }
}
