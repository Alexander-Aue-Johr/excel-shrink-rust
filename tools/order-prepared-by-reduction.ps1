<# .SYNOPSIS
Create an explicit study workbook list, strongest relative Default reduction first.
Only ZIP directory sizes and preparation metadata are read; Excel files are not regenerated.
#>
param(
    [Parameter(Mandatory=$true)][string]$PreparedDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Web.Extensions
Add-Type -ReferencedAssemblies System.Core,System.IO.Compression,System.IO.Compression.FileSystem,System.Web.Extensions -TypeDefinition @"
using System;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Collections.Generic;
using System.Text;
using System.Web.Script.Serialization;

public static class PreparedReductionOrder
{
    sealed class Workbook
    {
        public string Identity, Name, Bundle;
        public long Original, Default, Saved;
        public decimal Reduction;
    }
    static string Csv(string value) { return "\"" + value.Replace("\"", "\"\"") + "\""; }

    public static void Create(string prepared, string output)
    {
        var serializer = new JavaScriptSerializer { MaxJsonLength = Int32.MaxValue };
        var rows = new List<Workbook>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (string path in Directory.GetFiles(prepared, "*.zip"))
        {
            using (var zip = ZipFile.OpenRead(path))
            {
                var entry = zip.GetEntry("preparation.json");
                if (entry == null) throw new InvalidDataException("Missing preparation metadata: " + path);
                Dictionary<string, object> metadata;
                using (var reader = new StreamReader(entry.Open()))
                    metadata = serializer.DeserializeObject(reader.ReadToEnd()) as Dictionary<string, object>;
                string identity = Convert.ToString(metadata["identity"]);
                var variants = metadata["variants"] as object[];
                var variant = variants.Cast<Dictionary<string, object>>()
                    .Single(value => Convert.ToString(value["profile"]) == "default");
                var original = zip.GetEntry("original.xlsx");
                var shrunk = zip.GetEntry(Convert.ToString(variant["member"]));
                if (String.IsNullOrEmpty(identity) || !seen.Add(identity) || original == null ||
                    shrunk == null || original.Length <= 0)
                    throw new InvalidDataException("Invalid or duplicate preparation: " + path);
                rows.Add(new Workbook { Identity = identity, Name = Convert.ToString(metadata["name"]),
                    Bundle = path, Original = original.Length, Default = shrunk.Length,
                    Saved = original.Length - shrunk.Length,
                    Reduction = (decimal)(original.Length - shrunk.Length) / original.Length });
            }
        }
        if (rows.Count == 0) throw new InvalidDataException("No prepared workbooks.");
        var ordered = rows.OrderByDescending(row => row.Reduction)
            .ThenByDescending(row => row.Saved)
            .ThenBy(row => row.Identity, StringComparer.Ordinal).ToArray();
        Directory.CreateDirectory(output);
        string list = Path.Combine(output, "workbook-list.json");
        if (File.Exists(list)) throw new IOException("Use a new output directory.");
        File.WriteAllText(list, serializer.Serialize(ordered.Select(row => row.Identity).ToArray()));
        using (var csv = new StreamWriter(Path.Combine(output, "reduction-order.csv"), false, new UTF8Encoding(true)))
        {
            csv.WriteLine("rank,identity,source_member,original_xlsx_bytes,default_xlsx_bytes,saved_bytes,reduction_percent,source_bundle");
            int rank = 0;
            foreach (var row in ordered)
                csv.WriteLine(String.Join(",", new [] { (++rank).ToString(), Csv(row.Identity), Csv(row.Name),
                    row.Original.ToString(), row.Default.ToString(), row.Saved.ToString(),
                    (row.Reduction * 100).ToString("G", System.Globalization.CultureInfo.InvariantCulture), Csv(row.Bundle) }));
        }
        Console.WriteLine("Ranked " + ordered.Length + " preparations: relative Default reduction, saved bytes, identity.");
    }
}
"@
[PreparedReductionOrder]::Create([IO.Path]::GetFullPath($PreparedDirectory),[IO.Path]::GetFullPath($OutputDirectory))