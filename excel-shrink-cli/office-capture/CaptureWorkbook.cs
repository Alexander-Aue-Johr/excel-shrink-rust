using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Xml;

public static class CaptureWorkbook
{
    private static readonly HashSet<string> FollowingElements = new HashSet<string>(StringComparer.Ordinal) {
        "smartTags", "drawing", "drawingHF", "legacyDrawing", "legacyDrawingHF", "picture",
        "oleObjects", "controls", "webPublishItems", "tableParts", "extLst"
    };

    public static void HideErrorIndicators(string workbook)
    {
        Prepare(workbook, null, true, false);
    }

    // Only the disposable capture copy may be passed here. Open-time measurement
    // and shrink inputs retain their original workbook/view state.
    public static void PrepareCapture(string workbook, int initialWorksheet, bool hideErrorIndicators)
    {
        PrepareCapture(workbook, initialWorksheet, hideErrorIndicators, false);
    }

    /// <summary>Prepare a disposable comparison copy. Manual calculation suppresses the
    /// implicit open/save recalculation; an explicit full rebuild remains a separate job.</summary>
    public static void PrepareCapture(string workbook, int initialWorksheet, bool hideErrorIndicators, bool manualCalculation)
    {
        if (initialWorksheet < 1) throw new ArgumentOutOfRangeException("initialWorksheet");
        Prepare(workbook, initialWorksheet, hideErrorIndicators, manualCalculation);
    }

    /// <summary>Suppress implicit calculation without changing formulas, cached values or iteration settings.</summary>
    private static void SetManualCalculation(XmlDocument workbook)
    {
        var root = workbook.DocumentElement;
        var calculation = root["calcPr", root.NamespaceURI];
        if (calculation == null)
        {
            calculation = workbook.CreateElement(root.Prefix, "calcPr", root.NamespaceURI);
            var following = new HashSet<string>(StringComparer.Ordinal) {
                "oleSize", "customWorkbookViews", "pivotCaches", "smartTagPr", "smartTagTypes",
                "webPublishing", "fileRecoveryPr", "webPublishObjects", "extLst"
            };
            XmlNode next = null;
            foreach (XmlNode child in root.ChildNodes)
                if (child.NamespaceURI == root.NamespaceURI && following.Contains(child.LocalName)) { next = child; break; }
            root.InsertBefore(calculation, next);
        }
        calculation.SetAttribute("calcMode", "manual");
        calculation.SetAttribute("fullCalcOnLoad", "0");
        calculation.SetAttribute("forceFullCalc", "0");
        calculation.SetAttribute("calcOnSave", "0");
    }

    private sealed class InitialSelection
    {
        public XmlDocument Workbook;
        public HashSet<string> SheetParts;
        public string SelectedPart;
        public int ViewCount;
    }

    private static void Prepare(string workbook, int? initialWorksheet, bool hideErrorIndicators, bool manualCalculation)
    {
        var pending = workbook + ".prepared";
        try
        {
            using (var source = ZipFile.OpenRead(workbook))
            using (var file = new FileStream(pending, FileMode.CreateNew))
            using (var output = new ZipArchive(file, ZipArchiveMode.Create))
            {
                var worksheets = WorksheetParts(source);
                var selection = initialWorksheet.HasValue ? ReadInitialSelection(source, initialWorksheet.Value, worksheets) : null;
                if (manualCalculation) SetManualCalculation(selection.Workbook);
                foreach (var entry in source.Entries)
                {
                    var copy = output.CreateEntry(entry.FullName, CompressionLevel.Fastest);
                    copy.LastWriteTime = entry.LastWriteTime;
                    using (var input = entry.Open())
                    using (var target = copy.Open())
                    {
                        if (selection != null && entry.FullName == "xl/workbook.xml") selection.Workbook.Save(target);
                        else if (selection != null && selection.SheetParts.Contains(entry.FullName))
                            PrepareSheet(input, target, entry.FullName == selection.SelectedPart, selection.ViewCount,
                                hideErrorIndicators && worksheets.Contains(entry.FullName));
                        else if (hideErrorIndicators && worksheets.Contains(entry.FullName)) HideWorksheetErrorIndicators(input, target);
                        else input.CopyTo(target);
                    }
                }
            }
            File.Replace(pending, workbook, null);
        }
        finally { if (File.Exists(pending)) File.Delete(pending); }
    }

    private static XmlDocument ReadDocument(ZipArchive source, string name)
    {
        var entry = source.GetEntry(name);
        if (entry == null) throw new InvalidDataException("Workbook part is missing: " + name);
        var document = new XmlDocument { PreserveWhitespace = true, XmlResolver = null };
        using (var stream = entry.Open())
        using (var reader = XmlReader.Create(stream, new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null }))
            document.Load(reader);
        return document;
    }

    private static string RelationshipId(XmlElement sheet)
    {
        var id = sheet.GetAttribute("id", "http://schemas.openxmlformats.org/officeDocument/2006/relationships");
        if (id.Length == 0) id = sheet.GetAttribute("id", "http://purl.oclc.org/ooxml/officeDocument/relationships");
        // Excel can retain a veryHidden VBA module placeholder after XLS to
        // macro-free XLSX conversion. Its explicitly empty r:id has no part.
        if (id.Length == 0 && sheet.GetAttribute("state") == "veryHidden" &&
            (sheet.HasAttribute("id", "http://schemas.openxmlformats.org/officeDocument/2006/relationships") ||
             sheet.HasAttribute("id", "http://purl.oclc.org/ooxml/officeDocument/relationships"))) return null;
        if (id.Length == 0) throw new InvalidDataException("Worksheet relationship ID is missing.");
        return id;
    }

    private static string SheetPart(string target)
    {
        if (target.Length == 0 || target.IndexOf('\\') >= 0) throw new InvalidDataException("Invalid sheet relationship target.");
        var uri = new Uri(new Uri("http://capture.invalid/xl/workbook.xml"), target);
        if (uri.Scheme != "http" || uri.Host != "capture.invalid" || uri.Query.Length != 0 || uri.Fragment.Length != 0)
            throw new InvalidDataException("External or invalid sheet relationship target.");
        return Uri.UnescapeDataString(uri.AbsolutePath).TrimStart('/');
    }

    private static InitialSelection ReadInitialSelection(ZipArchive source, int index, HashSet<string> worksheets)
    {
        var workbook = ReadDocument(source, "xl/workbook.xml");
        var root = workbook.DocumentElement;
        var ns = root.NamespaceURI;
        if (root.LocalName != "workbook" || (ns != "http://schemas.openxmlformats.org/spreadsheetml/2006/main" &&
            ns != "http://purl.oclc.org/ooxml/spreadsheetml/main"))
            throw new InvalidDataException("Unsupported workbook XML root.");
        var namespaces = new XmlNamespaceManager(workbook.NameTable);
        namespaces.AddNamespace("s", ns);
        var sheets = root.SelectNodes("s:sheets/s:sheet", namespaces);
        if (index > sheets.Count) throw new InvalidDataException("Initial worksheet index is outside the workbook.");
        var selected = (XmlElement)sheets[index - 1];
        var state = selected.GetAttribute("state");
        if (state.Length != 0 && state != "visible") throw new InvalidDataException("Initial worksheet is hidden.");

        var relationships = ReadDocument(source, "xl/_rels/workbook.xml.rels");
        var partsById = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (XmlNode node in relationships.DocumentElement.ChildNodes)
        {
            var relation = node as XmlElement;
            if (relation == null || relation.LocalName != "Relationship") continue;
            if (relation.GetAttribute("TargetMode") == "External") continue;
            var id = relation.GetAttribute("Id");
            if (partsById.ContainsKey(id)) throw new InvalidDataException("Duplicate workbook relationship ID.");
            partsById.Add(id, relation.GetAttribute("Target"));
        }
        var sheetParts = new HashSet<string>(StringComparer.Ordinal);
        string selectedPart = null;
        foreach (XmlElement sheet in sheets)
        {
            string target;
            var relationshipId = RelationshipId(sheet);
            if (relationshipId == null) continue;
            if (!partsById.TryGetValue(relationshipId, out target)) throw new InvalidDataException("Worksheet relationship is missing.");
            var part = SheetPart(target);
            if (source.GetEntry(part) == null || !sheetParts.Add(part)) throw new InvalidDataException("Missing or duplicate worksheet part.");
            if (sheet == selected) selectedPart = part;
        }
        if (!worksheets.Contains(selectedPart)) throw new InvalidDataException("Initial sheet is not a worksheet.");

        var views = (XmlElement)root.SelectSingleNode("s:bookViews", namespaces);
        if (views == null)
        {
            views = workbook.CreateElement(root.Prefix, "bookViews", ns);
            root.InsertBefore(views, root.SelectSingleNode("s:sheets", namespaces));
        }
        if (views.SelectNodes("s:workbookView", namespaces).Count == 0)
            views.AppendChild(workbook.CreateElement(root.Prefix, "workbookView", ns));
        var workbookViews = views.SelectNodes("s:workbookView", namespaces);
        foreach (XmlElement view in workbookViews)
        {
            view.SetAttribute("activeTab", (index - 1).ToString(System.Globalization.CultureInfo.InvariantCulture));
        }
        return new InitialSelection { Workbook = workbook, SheetParts = sheetParts, SelectedPart = selectedPart, ViewCount = workbookViews.Count };
    }

    private static HashSet<string> WorksheetParts(ZipArchive archive)
    {
        var parts = new HashSet<string>(StringComparer.Ordinal);
        using (var stream = archive.GetEntry("[Content_Types].xml").Open())
        using (var reader = XmlReader.Create(stream))
        {
            while (reader.Read())
                if (reader.NodeType == XmlNodeType.Element && reader.LocalName == "Override" &&
                    reader.GetAttribute("ContentType") == "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml")
                    parts.Add(reader.GetAttribute("PartName").TrimStart('/'));
        }
        return parts;
    }

    public static void HideWorksheetErrorIndicators(Stream source, Stream destination)
    {
        PrepareSheet(source, destination, false, 0, true);
    }

    private static void PrepareSheet(Stream source, Stream destination, bool selected, int viewCount, bool hideErrorIndicators)
    {
        var settings = new XmlWriterSettings { Encoding = new UTF8Encoding(false), NewLineHandling = NewLineHandling.Entitize };
        using (var reader = XmlReader.Create(source))
        using (var writer = XmlWriter.Create(destination, settings))
        {
            reader.MoveToContent();
            if (reader.LocalName != "worksheet" && (viewCount == 0 ||
                (reader.LocalName != "chartsheet" && reader.LocalName != "dialogsheet" && reader.LocalName != "macrosheet")))
                throw new InvalidDataException("Expected a worksheet/chart sheet XML part.");
            var prefix = reader.Prefix;
            var ns = reader.NamespaceURI;
            writer.WriteStartElement(prefix, reader.LocalName, ns);
            writer.WriteAttributes(reader, true);
            var written = false;
            var viewsWritten = false;
            var empty = reader.IsEmptyElement;
            reader.Read();
            while (!empty && !(reader.NodeType == XmlNodeType.EndElement && reader.Depth == 0))
            {
                if (reader.EOF) throw new InvalidDataException("Incomplete worksheet XML.");
                if (reader.NodeType == XmlNodeType.Element && reader.Depth == 1 && reader.NamespaceURI == ns)
                {
                    if (viewCount > 0 && reader.LocalName == "sheetViews")
                    {
                        if (viewsWritten) throw new InvalidDataException("Duplicate or misplaced worksheet views.");
                        var document = new XmlDocument { PreserveWhitespace = true, XmlResolver = null };
                        var views = (XmlElement)document.ReadNode(reader);
                        NormalizeSheetViews(views, selected, viewCount);
                        views.WriteTo(writer);
                        viewsWritten = true;
                        continue;
                    }
                    if (viewCount > 0 && !viewsWritten && reader.LocalName != "sheetPr" && reader.LocalName != "dimension")
                    {
                        if (selected) WriteSheetViews(writer, prefix, ns, viewCount);
                        viewsWritten = true;
                    }
                    if (hideErrorIndicators && reader.LocalName == "ignoredErrors")
                    {
                        if (!written) WriteIgnoredErrors(writer, prefix, ns);
                        written = true;
                        reader.Skip();
                        continue;
                    }
                    if (hideErrorIndicators && !written && FollowingElements.Contains(reader.LocalName))
                    {
                        WriteIgnoredErrors(writer, prefix, ns);
                        written = true;
                    }
                }
                writer.WriteNode(reader, true);
            }
            if (viewCount > 0 && !viewsWritten && selected) WriteSheetViews(writer, prefix, ns, viewCount);
            if (hideErrorIndicators && !written) WriteIgnoredErrors(writer, prefix, ns);
            writer.WriteEndElement();
        }
    }

    private static void NormalizeSheetViews(XmlElement views, bool selected, int viewCount)
    {
        var ids = new HashSet<int>();
        foreach (XmlNode node in views.ChildNodes)
        {
            var view = node as XmlElement;
            if (view == null || view.LocalName != "sheetView" || view.NamespaceURI != views.NamespaceURI) continue;
            int id;
            if (!int.TryParse(view.GetAttribute("workbookViewId"), out id) || id < 0 || id >= viewCount || !ids.Add(id))
                throw new InvalidDataException("Invalid or duplicate worksheet workbookViewId.");
            view.SetAttribute("tabSelected", selected ? "1" : "0");
        }
        if (!selected) return;
        for (var id = 0; id < viewCount; id++)
        {
            if (ids.Contains(id)) continue;
            var view = views.OwnerDocument.CreateElement(views.Prefix, "sheetView", views.NamespaceURI);
            view.SetAttribute("workbookViewId", id.ToString(System.Globalization.CultureInfo.InvariantCulture));
            view.SetAttribute("tabSelected", "1");
            views.AppendChild(view);
        }
    }

    private static void WriteSheetViews(XmlWriter writer, string prefix, string ns, int count)
    {
        writer.WriteStartElement(prefix, "sheetViews", ns);
        for (var id = 0; id < count; id++)
        {
            writer.WriteStartElement(prefix, "sheetView", ns);
            writer.WriteAttributeString("workbookViewId", id.ToString(System.Globalization.CultureInfo.InvariantCulture));
            writer.WriteAttributeString("tabSelected", "1");
            writer.WriteEndElement();
        }
        writer.WriteEndElement();
    }

    private static void WriteIgnoredErrors(XmlWriter writer, string prefix, string ns)
    {
        // Warning triangles are Excel UI state, not cell content.
        writer.WriteStartElement(prefix, "ignoredErrors", ns);
        writer.WriteStartElement(prefix, "ignoredError", ns);
        writer.WriteAttributeString("sqref", "A1:XFD1048576");
        foreach (var flag in new[] { "numberStoredAsText", "twoDigitTextYear", "evalError", "formula",
            "formulaRange", "unlockedFormula", "emptyCellReference", "listDataValidation", "calculatedColumn" })
            writer.WriteAttributeString(flag, "1");
        writer.WriteEndElement();
        writer.WriteEndElement();
    }
}
