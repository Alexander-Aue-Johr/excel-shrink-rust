using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json;
using ExcelDna.Integration;

public static class DrawingBoundsCommands
{
    [ExcelCommand(Name = "DrawingBoundsCSharp_RunDrawingBoundsBatch")]
    public static string RunDrawingBoundsBatch(
        string fileListPath,
        string outputPath)
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

    private static void Run(string fileListPath, string outputPath)
    {
        dynamic excel = ExcelDnaUtil.Application;

        string[] paths = File.ReadAllLines(fileListPath)
            .Select(line => line.Trim())
            .Where(line => line.Length != 0)
            .ToArray();

        if (paths.Length == 0)
        {
            throw new InvalidOperationException(
                $"File list is empty: {fileListPath}");
        }

        dynamic workbooks = excel.Workbooks;

        var seenNames = new HashSet<string>(
            StringComparer.OrdinalIgnoreCase);

        var json = new StringBuilder();
        json.AppendLine("{");

        bool firstWorkbook = true;

        foreach (string path in paths)
        {
            dynamic? workbook = null;

            try
            {
                workbook = workbooks.Open(
                    path,
                    0,
                    true);

                string workbookName =
                    Convert.ToString(workbook.Name, CultureInfo.InvariantCulture)
                    ?? throw new InvalidOperationException(
                        $"Workbook.Name was null: {path}");

                if (!seenNames.Add(workbookName))
                {
                    throw new InvalidOperationException(
                        $"Duplicate workbook filename in corpus: {workbookName}");
                }

                if (!firstWorkbook)
                {
                    json.AppendLine(",");
                }

                firstWorkbook = false;

                json.Append("  ");
                json.Append(QuoteJson(workbookName));
                json.AppendLine(": {");

                dynamic worksheets = workbook.Worksheets;
                int count = Convert.ToInt32(
                    worksheets.Count,
                    CultureInfo.InvariantCulture);

                bool firstSheet = true;

                for (int index = 1; index <= count; index++)
                {
                    dynamic worksheet = worksheets.Item(index);

                    string sheetName =
                        Convert.ToString(
                            worksheet.Name,
                            CultureInfo.InvariantCulture)
                        ?? $"Sheet{index}";

                    var stopwatch = Stopwatch.StartNew();
                    var bounds =
                        VisibleDrawingBounds((object)worksheet);
                    double right = bounds.Right;
                    double bottom = bounds.Bottom;
                    stopwatch.Stop();

                    double durationMs = stopwatch.Elapsed.TotalMilliseconds;

                    if (!firstSheet)
                    {
                        json.AppendLine(",");
                    }

                    firstSheet = false;

                    json.Append("    ");
                    json.Append(QuoteJson(sheetName));
                    json.AppendLine(": {");

                    json.Append("      \"right\": ");
                    json.Append(InvariantFixed(right));
                    json.AppendLine(",");

                    json.Append("      \"bottom\": ");
                    json.Append(InvariantFixed(bottom));
                    json.AppendLine(",");

                    json.Append("      \"duration_ms\": ");
                    json.Append(InvariantFixed(durationMs));
                    json.AppendLine();
                    json.Append("    }");
                }

                json.AppendLine();
                json.Append("  }");

                workbook.Close(false);
                workbook = null;
            }
            finally
            {
                if (workbook is not null)
                {
                    try
                    {
                        workbook.Close(false);
                    }
                    catch
                    {
                    }
                }
            }
        }

        json.AppendLine();
        json.AppendLine("}");

        string? parent = Path.GetDirectoryName(outputPath);
        if (parent is null)
        {
            throw new InvalidOperationException(
                $"Output has no parent directory: {outputPath}");
        }

        Directory.CreateDirectory(parent);

        File.WriteAllText(
            outputPath,
            json.ToString(),
            new UTF8Encoding(encoderShouldEmitUTF8Identifier: false));
    }

    private static (double Right, double Bottom)
        VisibleDrawingBounds(object worksheetObject)
    {
        dynamic worksheet = worksheetObject;
        dynamic shapes = worksheet.Shapes;

        int count = Convert.ToInt32(
            shapes.Count,
            CultureInfo.InvariantCulture);

        double maxRight = 0.0;
        double maxBottom = 0.0;

        for (int index = 1; index <= count; index++)
        {
            dynamic shape = shapes.Item(index);

            int visible = Convert.ToInt32(
                shape.Visible,
                CultureInfo.InvariantCulture);

            if (visible == 0)
            {
                continue;
            }

            double left = Convert.ToDouble(
                shape.Left,
                CultureInfo.InvariantCulture);

            double top = Convert.ToDouble(
                shape.Top,
                CultureInfo.InvariantCulture);

            double width = Convert.ToDouble(
                shape.Width,
                CultureInfo.InvariantCulture);

            double height = Convert.ToDouble(
                shape.Height,
                CultureInfo.InvariantCulture);

            double angle =
                Convert.ToDouble(
                    shape.Rotation,
                    CultureInfo.InvariantCulture)
                * Math.PI
                / 180.0;

            double rotatedWidth =
                Math.Abs(width * Math.Cos(angle))
                + Math.Abs(height * Math.Sin(angle));

            double rotatedHeight =
                Math.Abs(width * Math.Sin(angle))
                + Math.Abs(height * Math.Cos(angle));

            double right =
                left
                + width / 2.0
                + rotatedWidth / 2.0;

            double bottom =
                top
                + height / 2.0
                + rotatedHeight / 2.0;

            maxRight = Math.Max(maxRight, right);
            maxBottom = Math.Max(maxBottom, bottom);
        }

        return (maxRight, maxBottom);
    }

    private static string InvariantFixed(double value)
    {
        return value.ToString(
            "0.000000",
            CultureInfo.InvariantCulture);
    }

    private static string QuoteJson(string value)
    {
        return JsonSerializer.Serialize(value);
    }
}
