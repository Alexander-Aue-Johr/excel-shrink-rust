using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Threading;

/// <summary>Native chart operations use the worker's retained workbook reference.</summary>
public sealed partial class CaptureWorkbookSession
{
    static object Item(object collection, object key)
    {
        return collection.GetType().InvokeMember("Item", BindingFlags.GetProperty | BindingFlags.InvokeMethod,
            null, collection, new object[] { key }, AutomationCulture);
    }

    public object ChartSheets()
    {
        RequireCurrentWorkbook();
        object charts = null;
        try
        {
            VerifyProcess();
            charts = Get(currentWorkbook, "Charts");
            var result = new List<object>();
            int count = Convert.ToInt32(Get(charts, "Count"));
            for (int index = 1; index <= count; index++)
            {
                object chart = Item(charts, index);
                try
                {
                    string name = Convert.ToString(Get(chart, "Name"));
                    result.Add(new { sheet = name, name = name, kind = "chart_sheet" });
                }
                finally { Release(chart); }
            }
            return result;
        }
        catch { faulted = true; throw; }
        finally { Release(charts); }
    }

    /// <summary>Normalize chart-sheet viewport or export an embedded chart, restoring transient view state.</summary>
    public object ExportChart(string sheetName, string chartName, bool chartSheet, int? chartIndex, string output)
    {
        RequireCurrentWorkbook();
        string destination = Path.GetFullPath(output);
        if (File.Exists(destination)) throw new IOException("Chart output already exists.");
        object sheets = null, sheet = null, objects = null, chartObject = null, chart = null;
        object active = null, windows = null, window = null, page = null, area = null;
        object visibility = null, sizeWithWindow = null, zoom = null, chartSize = null;
        var view = new Dictionary<string, object>();
        string phase = "workbook identity";
        try
        {
            VerifyProcess();
            active = Get(currentWorkbook, "ActiveSheet");
            phase = "sheet selection";
            sheets = Get(currentWorkbook, chartSheet ? "Charts" : "Worksheets");
            sheet = Item(sheets, chartSheet ? chartName : sheetName);
            visibility = Get(sheet, "Visible");
            var geometry = new Dictionary<string, object>();
            if (chartSheet)
            {
                chart = sheet; // Release this shared RCW only once below.
                windows = Get(currentWorkbook, "Windows");
                window = Item(windows, 1);
                foreach (string property in new[] { "WindowState", "Width", "Height", "Left", "Top" })
                    view[property] = Get(window, property);
                sizeWithWindow = Get(chart, "SizeWithWindow");
                Set(chart, "SizeWithWindow", true);
                Set(window, "WindowState", -4143);
                Set(window, "Left", 0);
                Set(window, "Top", 0);
                Set(window, "Width", 1000);
                Set(window, "Height", 750);
            }
            else
            {
                phase = "embedded chart collection";
                objects = sheet.GetType().InvokeMember("ChartObjects",
                    BindingFlags.GetProperty | BindingFlags.InvokeMethod | BindingFlags.OptionalParamBinding,
                    null, sheet, new object[] { Type.Missing }, AutomationCulture);
                phase = "embedded chart selection";
                if (chartIndex.HasValue)
                {
                    if (chartIndex.Value < 0 || chartIndex.Value >= Convert.ToInt32(Get(objects, "Count")))
                        throw new ArgumentException("Invalid chart collection index.");
                    chartObject = Item(objects, chartIndex.Value + 1);
                    if (Convert.ToString(Get(chartObject, "Name")) != chartName)
                        throw new InvalidOperationException("Chart collection order changed since enumeration.");
                }
                else { chartObject = Item(objects, chartName); }
                phase = "embedded chart geometry";
                foreach (string property in new[] { "Top", "Left", "Width", "Height" })
                    geometry[property.ToLowerInvariant()] = Get(chartObject, property);
                geometry["export_method"] = "native-existing-process";
                if (Convert.ToDouble(geometry["width"]) == 0 || Convert.ToDouble(geometry["height"]) == 0)
                    return new { non_rendering = true, reason = "zero-area-chart", geometry = geometry };
                phase = "embedded chart reference";
                chart = Get(chartObject, "Chart");
            }
            phase = "sheet activation";
            Set(sheet, "Visible", -1);
            Invoke(sheet, "Activate", new object[0]);
            if (chartSheet)
            {
                zoom = Get(window, "Zoom");
                Set(window, "Zoom", 100);
                page = Get(chart, "PageSetup");
                chartSize = Get(page, "ChartSize");
                Set(page, "ChartSize", 1);
                area = Get(chart, "ChartArea");
                geometry["visibility"] = visibility;
                geometry["page_chart_size"] = chartSize;
            }
            phase = "image export";
            ExportChartImage(chart, destination);
            if (chartSheet)
            {
                geometry["width"] = Get(area, "Width");
                geometry["height"] = Get(area, "Height");
            }
            VerifyProcess();
            return new { path = destination, geometry = geometry };
        }
        catch (Exception error)
        {
            faulted = true;
            throw new InvalidOperationException("Native chart " + phase + " failed: " + error.GetBaseException().Message);
        }
        finally
        {
            try
            {
                if (chartSize != null) Set(page, "ChartSize", chartSize);
                if (zoom != null) Set(window, "Zoom", zoom);
                if (sizeWithWindow != null) Set(chart, "SizeWithWindow", sizeWithWindow);
                foreach (string property in new[] { "Width", "Height", "Left", "Top", "WindowState" })
                    if (view.ContainsKey(property)) Set(window, property, view[property]);
                if (active != null) Invoke(active, "Activate", new object[0]);
                if (visibility != null) Set(sheet, "Visible", visibility);
            }
            catch { faulted = true; throw; }
            finally
            {
                Release(area); Release(page); Release(window); Release(windows);
                if (!chartSheet) Release(chart);
                Release(chartObject); Release(objects); Release(sheet); Release(sheets); Release(active);
            }
        }
    }

    static void ExportChartImage(object chart, string destination)
    {
        Invoke(chart, "Refresh", new object[0]);
        for (int attempt = 0; attempt < 10; attempt++)
        {
            Thread.Sleep(200);
            bool exported = Convert.ToBoolean(Invoke(chart, "Export", new object[] { destination, "PNG", false }));
            if (exported && File.Exists(destination) && new FileInfo(destination).Length > 0) return;
            Invoke(chart, "Refresh", new object[0]);
        }
        throw new IOException("Chart export did not produce a nonempty image.");
    }
}
