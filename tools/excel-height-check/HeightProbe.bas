Option Explicit

Private Declare PtrSafe Function QueryPerformanceCounter Lib "kernel32" (ByRef value As Currency) As Long
Private Declare PtrSafe Function QueryPerformanceFrequency Lib "kernel32" (ByRef value As Currency) As Long
Private Declare PtrSafe Function GetCurrentProcessId Lib "kernel32" () As Long

Private Function Tick() As Currency
    QueryPerformanceCounter Tick
End Function

Private Function Milliseconds(ByVal started As Currency) As Double
    Dim frequency As Currency, finished As Currency
    QueryPerformanceFrequency frequency
    QueryPerformanceCounter finished
    Milliseconds = (finished - started) * 1000# / frequency
End Function

Public Function Collect(ByVal workbookName As String, Optional ByVal addressPlan As String = "") As Variant
    Dim started As Currency, t As Currency
    Dim wb As Workbook, ws As Worksheet, used As Range
    Dim result() As Variant, n As Long, i As Long, addresses As Variant
    started = Tick()
    Set wb = Application.Workbooks(workbookName)
    n = wb.Worksheets.Count
    ReDim result(0 To n, 0 To 12)
    addresses = Split(addressPlan, vbLf)
    For i = 1 To n
        Set ws = wb.Worksheets(i)
        t = Tick()
        If Len(addressPlan) = 0 Then
            Set used = ws.UsedRange
        Else
            Set used = ws.Range(CStr(addresses(i - 1)))
        End If
        result(i - 1, 1) = Milliseconds(t)
        t = Tick()
        result(i - 1, 0) = used.Height
        result(i - 1, 2) = Milliseconds(t)
        result(i - 1, 3) = used.Address
        result(i - 1, 4) = ws.Name
        result(i - 1, 5) = used.Rows.Count
        result(i - 1, 6) = used.Columns.Count
        result(i - 1, 7) = used.Row
        result(i - 1, 8) = used.Column
        result(i - 1, 9) = used.Width
        result(i - 1, 10) = ws.Visible
        result(i - 1, 11) = ws.StandardHeight
        result(i - 1, 12) = ws.StandardWidth
        Set used = Nothing
        Set ws = Nothing
    Next i
    result(n, 0) = Milliseconds(started)
    result(n, 1) = GetCurrentProcessId()
    result(n, 2) = n
    Collect = result
End Function

Private Function JsonNumber(ByVal value As Double) As String
    JsonNumber = Replace$(CStr(value), ",", ".")
End Function

Public Function CollectToJson(ByVal workbookName As String, ByVal destination As String) As Variant
    Dim started As Currency, writing As Currency, result As Variant
    Dim summary(0 To 3) As Double, f As Integer, i As Long, n As Long, line As String
    started = Tick()
    result = Collect(workbookName)
    n = UBound(result, 1)
    writing = Tick()
    f = FreeFile
    On Error GoTo WriteFailed
    Open destination For Output As #f
    Print #f, "["
    For i = 0 To n - 1
        line = "{""sheet_index"":" & CStr(i + 1) & ",""height_points"":" & JsonNumber(result(i, 0)) & "}"
        If i < n - 1 Then line = line & ","
        Print #f, line
    Next i
    Print #f, "]"
    Close #f
    summary(0) = result(n, 0)
    summary(1) = Milliseconds(writing)
    summary(2) = Milliseconds(started)
    summary(3) = GetCurrentProcessId()
    CollectToJson = summary
    Exit Function
WriteFailed:
    Dim code As Long, message As String
    code = Err.Number
    message = Err.Description
    Close #f
    Err.Raise code, "HeightProbe.CollectToJson", message
End Function
