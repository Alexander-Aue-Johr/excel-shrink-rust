using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using Excel = Microsoft.Office.Interop.Excel;
using Office = Microsoft.Office.Core;

namespace DrawingBoundsVsto
{
    [ComVisible(true)]
    [Guid("D1E2F4A6-DC78-4E6E-967C-3BC31620F8D2")]
    [InterfaceType(ComInterfaceType.InterfaceIsIDispatch)]
    public interface IDrawingBoundsAutomation
    {
        string RunDrawingBoundsBatch(string fileListPath, string outputPath);
    }

    [ComVisible(true)]
    [Guid("0A50353B-F5B8-4DF8-B781-6EB7BE304B00")]
    [ClassInterface(ClassInterfaceType.None)]
    public sealed class DrawingBoundsAutomation : IDrawingBoundsAutomation
    {
        private readonly Excel.Application application;

        public DrawingBoundsAutomation(Excel.Application application)
        {
            this.application = application;
        }

        public string RunDrawingBoundsBatch(string fileListPath, string outputPath)
        {
            try
            {
                Run(fileListPath, outputPath);
                return "OK";
            }
            catch (Exception exception)
            {
                return "ERROR: " + exception;
            }
        }

        private void Run(string fileListPath, string outputPath)
        {
            string[] paths = File.ReadAllLines(fileListPath)
                .Select(line => line.Trim())
                .Where(line => line.Length != 0)
                .ToArray();

            if (paths.Length == 0)
            {
                throw new InvalidOperationException("File list is empty: " + fileListPath);
            }

            var seenNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var json = new StringBuilder();
            json.AppendLine("{");
            bool firstWorkbook = true;

            foreach (string path in paths)
            {
                Excel.Workbook workbook = null;

                try
                {
                    workbook = application.Workbooks.Open(
                        Filename: path,
                        UpdateLinks: 0,
                        ReadOnly: true);

                    string workbookName = workbook.Name;
                    if (!seenNames.Add(workbookName))
                    {
                        throw new InvalidOperationException(
                            "Duplicate workbook filename in corpus: " + workbookName);
                    }

                    if (!firstWorkbook)
                    {
                        json.AppendLine(",");
                    }
                    firstWorkbook = false;
                    json.Append("  ").Append(QuoteJson(workbookName)).AppendLine(": {");

                    Excel.Worksheets worksheets = workbook.Worksheets;
                    bool firstSheet = true;

                    for (int index = 1; index <= worksheets.Count; index++)
                    {
                        Excel.Worksheet worksheet = (Excel.Worksheet)worksheets.Item[index];
                        string sheetName = worksheet.Name;

                        var stopwatch = Stopwatch.StartNew();
                        Tuple<double, double> bounds = VisibleDrawingBounds(worksheet);
                        stopwatch.Stop();

                        if (!firstSheet)
                        {
                            json.AppendLine(",");
                        }
                        firstSheet = false;

                        json.Append("    ").Append(QuoteJson(sheetName)).AppendLine(": {");
                        json.Append("      \"right\": ").Append(InvariantFixed(bounds.Item1)).AppendLine(",");
                        json.Append("      \"bottom\": ").Append(InvariantFixed(bounds.Item2)).AppendLine(",");
                        json.Append("      \"duration_ms\": ").Append(InvariantFixed(stopwatch.Elapsed.TotalMilliseconds)).AppendLine();
                        json.Append("    }");
                    }

                    json.AppendLine();
                    json.Append("  }");
                    workbook.Close(false);
                    workbook = null;
                }
                finally
                {
                    if (workbook != null)
                    {
                        try { workbook.Close(false); } catch { }
                    }
                }
            }

            json.AppendLine();
            json.AppendLine("}");

            string parent = Path.GetDirectoryName(outputPath);
            if (String.IsNullOrEmpty(parent))
            {
                throw new InvalidOperationException("Output has no parent directory: " + outputPath);
            }

            Directory.CreateDirectory(parent);
            File.WriteAllText(outputPath, json.ToString(), new UTF8Encoding(false));
        }

        private static Tuple<double, double> VisibleDrawingBounds(Excel.Worksheet worksheet)
        {
            Excel.Shapes shapes = worksheet.Shapes;
            double maxRight = 0.0;
            double maxBottom = 0.0;

            for (int index = 1; index <= shapes.Count; index++)
            {
                Excel.Shape shape = shapes.Item(index);
                if (shape.Visible == Office.MsoTriState.msoFalse)
                {
                    continue;
                }

                double left = shape.Left;
                double top = shape.Top;
                double width = shape.Width;
                double height = shape.Height;
                double angle = shape.Rotation * Math.PI / 180.0;
                double rotatedWidth = Math.Abs(width * Math.Cos(angle)) + Math.Abs(height * Math.Sin(angle));
                double rotatedHeight = Math.Abs(width * Math.Sin(angle)) + Math.Abs(height * Math.Cos(angle));
                double right = left + width / 2.0 + rotatedWidth / 2.0;
                double bottom = top + height / 2.0 + rotatedHeight / 2.0;
                maxRight = Math.Max(maxRight, right);
                maxBottom = Math.Max(maxBottom, bottom);
            }

            return Tuple.Create(maxRight, maxBottom);
        }

        private static string InvariantFixed(double value)
        {
            return value.ToString("0.000000", CultureInfo.InvariantCulture);
        }

        private static string QuoteJson(string value)
        {
            var builder = new StringBuilder();
            builder.Append('"');

            foreach (char c in value)
            {
                switch (c)
                {
                    case '\\': builder.Append("\\\\"); break;
                    case '"': builder.Append("\\\""); break;
                    case '\r': builder.Append("\\r"); break;
                    case '\n': builder.Append("\\n"); break;
                    case '\t': builder.Append("\\t"); break;
                    default:
                        if (c < 0x20)
                        {
                            builder.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        }
                        else
                        {
                            builder.Append(c);
                        }
                        break;
                }
            }

            builder.Append('"');
            return builder.ToString();
        }
    }
}
