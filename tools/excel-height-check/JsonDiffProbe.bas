Attribute VB_Name = "JsonDiffProbe"
' Managed by patch_excel_json_diff.py; separate from the regression HeightProbe.
Option Explicit

Public Function JsonDiffVersion() As String
    JsonDiffVersion = "excel-json-diff-v1-b5eafbf3f126097b"
End Function

Private Function Num(ByVal value As Double) As String
    Num = Trim$(Str$(value))
    If Left$(Num, 1) = "." Then Num = "0" & Num
    If Left$(Num, 2) = "-." Then Num = "-0" & Mid$(Num, 2)
End Function

Private Function Quoted(ByVal value As String) As String
    Dim i As Long, n As Long, ch As String, result As String
    result = Chr$(34)
    For i = 1 To Len(value)
        ch = Mid$(value, i, 1)
        n = AscW(ch)
        If n < 0 Then n = n + 65536
        Select Case n
            Case 34, 92
                result = result & Chr$(92) & ch
            Case 32 To 126
                result = result & ch
            Case Else
                result = result & "\u" & Right$("0000" & Hex$(n), 4)
        End Select
    Next i
    Quoted = result & Chr$(34)
End Function

Private Function RangeInfo(ByVal area As Range) As String
    RangeInfo = "{""address"":" & Quoted(area.Address(True, True, xlA1, False)) & _
        ",""first_row"":" & CStr(area.Row) & ",""last_row"":" & CStr(area.Row + area.Rows.Count - 1) & _
        ",""first_column"":" & CStr(area.Column) & ",""last_column"":" & CStr(area.Column + area.Columns.Count - 1) & _
        ",""rows"":" & CStr(area.Rows.Count) & ",""columns"":" & CStr(area.Columns.Count) & _
        ",""height_points"":" & Num(area.Height) & ",""width_points"":" & Num(area.Width) & "}"
End Function

' Called before scanning rows: preserve each workbook's own initial UsedRange.
Public Function Describe(ByVal workbookName As String) As String
    Dim wb As Workbook, ws As Worksheet, area As Range, i As Long, result As String
    Set wb = Application.Workbooks(workbookName)
    result = "["
    For i = 1 To wb.Worksheets.Count
        Set ws = wb.Worksheets(i)
        Set area = ws.UsedRange
        If i > 1 Then result = result & ","
        result = result & "{""index"":" & CStr(i) & ",""name"":" & Quoted(ws.Name) & _
            ",""visibility"":" & CStr(ws.Visible) & ",""used_range"":" & RangeInfo(area) & _
            ",""standard_height_points"":" & Num(ws.StandardHeight) & _
            ",""standard_column_width"":" & Num(ws.StandardWidth) & _
            ",""worksheet_row_count"":" & CStr(ws.Rows.Count) & "}"
    Next i
    Describe = result & "]"
End Function

Public Function MatchedRange(ByVal workbookName As String, ByVal sheetIndex As Long, ByVal address As String) As String
    Dim area As Range
    Set area = Application.Workbooks(workbookName).Worksheets(sheetIndex).Range(address)
    MatchedRange = RangeInfo(area)
End Function

Private Function RowRun(ByVal firstRow As Long, ByVal lastRow As Long, ByVal points As Double, ByVal isHidden As Boolean) As String
    Dim flag As String
    If isHidden Then flag = "1" Else flag = "0"
    RowRun = CStr(firstRow) & vbTab & CStr(lastRow) & vbTab & Num(points) & vbTab & flag
End Function

' Read EVERY row inside Excel. A multi-row RowHeight is NOT a homogeneity test.
' TSV: inclusive first row, inclusive last row, effective height in points, hidden.
' Run-length encoding is exact, not sampling. Batch boundaries are merged by PS.
Public Function RowsBatch(ByVal workbookName As String, ByVal sheetIndex As Long, ByVal firstRow As Long, ByVal count As Long) As String
    Dim ws As Worksheet, row As Range, r As Long, lastRow As Long, runStart As Long
    Dim height As Double, previousHeight As Double, hidden As Boolean, previousHidden As Boolean
    Dim parts() As String, n As Long
    Set ws = Application.Workbooks(workbookName).Worksheets(sheetIndex)
    If count < 1 Or count > 65536 Or firstRow < 1 Then Err.Raise 5, , "Invalid row batch"
    lastRow = firstRow + count - 1
    If lastRow > ws.Rows.Count Then Err.Raise 5, , "Row batch outside worksheet"
    ReDim parts(0 To count - 1)
    runStart = firstRow
    For r = firstRow To lastRow
        Set row = ws.Rows(r)
        height = CDbl(row.Height)
        hidden = CBool(row.Hidden)
        If r > firstRow Then
            If height <> previousHeight Or hidden <> previousHidden Then
                parts(n) = RowRun(runStart, r - 1, previousHeight, previousHidden)
                n = n + 1
                runStart = r
            End If
        End If
        previousHeight = height
        previousHidden = hidden
    Next r
    parts(n) = RowRun(runStart, lastRow, previousHeight, previousHidden)
    ReDim Preserve parts(0 To n)
    RowsBatch = Join(parts, vbLf) & vbLf
End Function

' This fixture is created and discarded in memory; no source workbook is touched.
Public Function SelfTest() As String
    Dim wb As Workbook, ws As Worksheet, actual As String, expected As String
    Dim h1 As Double, h2 As Double, h3 As Double, code As Long, message As String
    On Error GoTo Failed
    Set wb = Application.Workbooks.Add(xlWBATWorksheet)
    Set ws = wb.Worksheets(1)
    ws.Rows(1).RowHeight = 12
    ws.Rows(2).RowHeight = 24
    ws.Rows(3).Hidden = True
    h1 = ws.Rows(1).Height: h2 = ws.Rows(2).Height: h3 = ws.Rows(3).Height
    expected = RowRun(1, 1, h1, False) & vbLf & RowRun(2, 2, h2, False) & vbLf & RowRun(3, 3, h3, True) & vbLf
    actual = RowsBatch(wb.Name, 1, 1, 3)
    If actual <> expected Then Err.Raise vbObjectError + 720, , "VBA row-batch self-test failed"
    actual = RowsBatch(wb.Name, 1, 2, 1)
    If actual <> RowRun(2, 2, h2, False) & vbLf Then Err.Raise vbObjectError + 721, , "VBA offset self-test failed"
    wb.Close False
    SelfTest = "ok"
    Exit Function
Failed:
    code = Err.Number: message = Err.Description
    On Error Resume Next
    If Not wb Is Nothing Then wb.Close False
    On Error GoTo 0
    Err.Raise code, "JsonDiffProbe.SelfTest", message
End Function
