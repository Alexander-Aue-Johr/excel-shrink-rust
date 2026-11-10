param(
    [Parameter(Mandatory=$true)][int]$PowerPointPid,
    [Parameter(Mandatory=$true)][string]$Session
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -ReferencedAssemblies UIAutomationClient, UIAutomationTypes -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using System.Windows.Automation;

public static class GraphConversionDialog
{
    public static void Watch(int processId, string session)
    {
        var process = new PropertyCondition(AutomationElement.ProcessIdProperty, processId);
        var reported = new HashSet<string>();
        while (!File.Exists(Path.Combine(session, "stop")))
        {
            if (File.Exists(Path.Combine(session, "convert-active")))
            {
                try
                {
                    var windows = AutomationElement.RootElement.FindAll(TreeScope.Children, process);
                    foreach (AutomationElement window in windows)
                    {
                        bool conversionQuestion = false;
                        bool unsupportedChart = false;
                        var descriptions = new List<string>();
                        var labels = window.FindAll(TreeScope.Descendants,
                            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Text));
                        foreach (AutomationElement label in labels)
                        {
                            string text = label.Current.Name;
                            descriptions.Add(text);
                            if (text.Contains("Do you want to convert this chart?") &&
                                text.Contains("newest format"))
                                conversionQuestion = true;
                            if (text == "One or more charts in this document cannot be converted to the current Microsoft Office chart format.")
                                unsupportedChart = true;
                        }
                        if (unsupportedChart)
                        {
                            var acknowledge = window.FindFirst(TreeScope.Descendants, new AndCondition(
                                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Button),
                                new PropertyCondition(AutomationElement.NameProperty, "OK")));
                            object acknowledgePattern;
                            if (acknowledge != null && acknowledge.TryGetCurrentPattern(InvokePattern.Pattern, out acknowledgePattern))
                            {
                                ((InvokePattern)acknowledgePattern).Invoke();
                                File.WriteAllText(Path.Combine(session, "unsupported-chart"), "PowerPoint cannot convert this chart format.");
                            }
                            continue;
                        }
                        if (!conversionQuestion)
                        {
                            string description = string.Join(" | ", descriptions);
                            if (reported.Count < 20 && reported.Add(description))
                                Console.WriteLine("Unmatched PowerPoint UI: " + description);
                            continue;
                        }
                        var button = window.FindFirst(TreeScope.Descendants, new AndCondition(
                            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Button),
                            new PropertyCondition(AutomationElement.NameProperty, "Convert")));
                        object pattern;
                        if (button != null && button.TryGetCurrentPattern(InvokePattern.Pattern, out pattern))
                            ((InvokePattern)pattern).Invoke();
                    }
                }
                catch (ElementNotAvailableException)
                {
                    // A converted chart closes its dialog during enumeration.
                    // Re-enumerate the current windows on the next iteration.
                }
            }
            Thread.Sleep(100);
        }
    }
}
'@
[GraphConversionDialog]::Watch($PowerPointPid, $Session)
