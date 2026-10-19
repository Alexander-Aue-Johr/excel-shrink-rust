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
        var pending = workbook + ".prepared";
        try
        {
            using (var source = ZipFile.OpenRead(workbook))
            using (var file = new FileStream(pending, FileMode.CreateNew))
            using (var output = new ZipArchive(file, ZipArchiveMode.Create))
            {
                var worksheets = WorksheetParts(source);
                foreach (var entry in source.Entries)
                {
                    var copy = output.CreateEntry(entry.FullName, CompressionLevel.Fastest);
                    copy.LastWriteTime = entry.LastWriteTime;
                    using (var input = entry.Open())
                    using (var target = copy.Open())
                    {
                        if (worksheets.Contains(entry.FullName)) HideWorksheetErrorIndicators(input, target);
                        else input.CopyTo(target);
                    }
                }
            }
            File.Replace(pending, workbook, null);
        }
        finally { if (File.Exists(pending)) File.Delete(pending); }
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
        var settings = new XmlWriterSettings { Encoding = new UTF8Encoding(false), NewLineHandling = NewLineHandling.Entitize };
        using (var reader = XmlReader.Create(source))
        using (var writer = XmlWriter.Create(destination, settings))
        {
            reader.MoveToContent();
            if (reader.LocalName != "worksheet") throw new InvalidDataException("Expected a worksheet XML part.");
            var prefix = reader.Prefix;
            var ns = reader.NamespaceURI;
            writer.WriteStartElement(prefix, "worksheet", ns);
            writer.WriteAttributes(reader, true);
            var written = false;
            var empty = reader.IsEmptyElement;
            reader.Read();
            while (!empty && !(reader.NodeType == XmlNodeType.EndElement && reader.Depth == 0))
            {
                if (reader.EOF) throw new InvalidDataException("Incomplete worksheet XML.");
                if (reader.NodeType == XmlNodeType.Element && reader.Depth == 1 && reader.NamespaceURI == ns)
                {
                    if (reader.LocalName == "ignoredErrors")
                    {
                        if (!written) WriteIgnoredErrors(writer, prefix, ns);
                        written = true;
                        reader.Skip();
                        continue;
                    }
                    if (!written && FollowingElements.Contains(reader.LocalName))
                    {
                        WriteIgnoredErrors(writer, prefix, ns);
                        written = true;
                    }
                }
                writer.WriteNode(reader, true);
            }
            if (!written) WriteIgnoredErrors(writer, prefix, ns);
            writer.WriteEndElement();
        }
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
