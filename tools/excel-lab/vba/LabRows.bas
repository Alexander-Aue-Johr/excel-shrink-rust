Attribute VB_Name = "LabRows"
Option Explicit

' Retains the existing exact row probe, but performs all batches inside Excel.
Public Function Collect(ByVal ws As Worksheet, ByVal lastRow As Long, ByVal batchRows As Long, ByVal scope As String) As String
    Dim first As Long, count As Long, block As String, lines As Variant, line As Variant, fields As Variant
    Dim pending As Boolean, r1 As Long, r2 As Long, h As String, hidden As String
    Dim parts As New Collection, nextFrom As Long
    nextFrom = 1
    For first = 1 To lastRow Step batchRows
        count = Application.Min(batchRows, lastRow - first + 1)
        block = JsonDiffProbe.RowsBatch(ws.Parent.Name, WorksheetIndex(ws), first, count)
        lines = Split(block, vbLf)
        For Each line In lines
            If Len(line) > 0 Then
                fields = Split(CStr(line), vbTab)
                If UBound(fields) <> 3 Then Err.Raise 5, , "Malformed row batch"
                If CLng(fields(0)) <> nextFrom Then Err.Raise 5, , "Gap/overlap in row batch"
                If CLng(fields(1)) < nextFrom Or CLng(fields(1)) > lastRow Then Err.Raise 5, , "Invalid row endpoint"
                nextFrom = CLng(fields(1)) + 1
                If pending And h = CStr(fields(2)) And hidden = CStr(fields(3)) Then
                    r2 = CLng(fields(1))
                Else
                    If pending Then parts.Add RunJson(r1, r2, h, hidden)
                    r1 = CLng(fields(0)): r2 = CLng(fields(1)): h = CStr(fields(2)): hidden = CStr(fields(3)): pending = True
                End If
            End If
        Next
        LabIO.Progress "row scan sheet=" & ws.Name & " through=" & CStr(first + count - 1) & "/" & CStr(lastRow)
    Next
    If nextFrom <> lastRow + 1 Or Not pending Then Err.Raise 5, , "Incomplete row collection"
    parts.Add RunJson(r1, r2, h, hidden)
    Collect = "{""scope"":" & LabIO.J(scope) & ",""first_row"":1,""last_row"":" & CStr(lastRow) & _
              ",""measured_rows"":" & CStr(lastRow) & ",""runs"":" & LabIO.JoinJson(parts) & "}"
End Function
Private Function RunJson(ByVal a As Long, ByVal b As Long, ByVal height As String, ByVal hidden As String) As String
    RunJson = "{""from"":" & CStr(a) & ",""to"":" & CStr(b) & ",""height_points"":" & height & ",""hidden"":" & LabIO.B(hidden = "1") & "}"
End Function
Private Function WorksheetIndex(ByVal ws As Worksheet) As Long
    Dim i As Long
    For i = 1 To ws.Parent.Worksheets.Count
        If ws.Parent.Worksheets(i) Is ws Then WorksheetIndex = i: Exit Function
    Next
    Err.Raise 5, , "Worksheet not found"
End Function
