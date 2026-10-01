Attribute VB_Name = "LabRanges"
Option Explicit

Public Function RangeJson(ByVal rg As Range) As String
    RangeJson = "{""address"":" & LabIO.J(rg.Address(True, True, xlA1, False)) & _
        ",""first_row"":" & CStr(rg.Row) & ",""last_row"":" & CStr(rg.Row + rg.Rows.Count - 1) & _
        ",""first_column"":" & CStr(rg.Column) & ",""last_column"":" & CStr(rg.Column + rg.Columns.Count - 1) & _
        ",""rows"":" & CStr(rg.Rows.Count) & ",""columns"":" & CStr(rg.Columns.Count) & _
        ",""height_points"":" & LabIO.N(rg.Height) & ",""width_points"":" & LabIO.N(rg.Width) & "}"
End Function
Public Function DescribeSheet(ByVal ws As Worksheet) As String
    Dim rg As Range: Set rg = ws.UsedRange
    DescribeSheet = "{""name"":" & LabIO.J(ws.Name) & ",""index"":" & CStr(ws.Index) & _
        ",""visibility"":" & CStr(ws.Visible) & ",""used_range"":" & RangeJson(rg) & _
        ",""standard_height_points"":" & LabIO.N(ws.StandardHeight) & _
        ",""standard_column_width"":" & LabIO.N(ws.StandardWidth) & "}"
End Function
' One external call, one UsedRange/Height read per worksheet; no row enumeration.
Public Function ExportFast(ByVal workbookName As String, ByVal destination As String) As String
    Dim wb As Workbook, ws As Worksheet, entries As New Collection
    Set wb = Application.Workbooks(workbookName)
    For Each ws In wb.Worksheets: entries.Add DescribeSheet(ws): Next
    LabIO.SaveText destination, LabIO.JoinJson(entries)
    ExportFast = "ok"
End Function
Public Function AddressUnion(ByVal a As Range, ByVal b As Range) As String
    Dim r1 As Long, r2 As Long, c1 As Long, c2 As Long
    r1 = Application.Min(a.Row, b.Row): c1 = Application.Min(a.Column, b.Column)
    r2 = Application.Max(a.Row + a.Rows.Count - 1, b.Row + b.Rows.Count - 1)
    c2 = Application.Max(a.Column + a.Columns.Count - 1, b.Column + b.Columns.Count - 1)
    AddressUnion = a.Worksheet.Range(a.Worksheet.Cells(r1, c1), a.Worksheet.Cells(r2, c2)).Address
End Function
