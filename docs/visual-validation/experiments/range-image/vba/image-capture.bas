Attribute VB_Name = "RangeImageCapture"
Option Explicit

#If VBA7 Then
Private Declare PtrSafe Function QueryPerformanceCounter Lib "kernel32" (ByRef value As Currency) As Long
Private Declare PtrSafe Function QueryPerformanceFrequency Lib "kernel32" (ByRef value As Currency) As Long
#Else
Private Declare Function QueryPerformanceCounter Lib "kernel32" (ByRef value As Currency) As Long
Private Declare Function QueryPerformanceFrequency Lib "kernel32" (ByRef value As Currency) As Long
#End If

Private mFrequency As Currency

Public Sub RunRangeImageCaptureBatch( _
    ByVal planPath As String, _
    ByVal outputRoot As String, _
    ByVal resultPath As String)

    Dim previousDisplayAlerts As Boolean
    Dim previousEnableEvents As Boolean

    previousDisplayAlerts = Application.DisplayAlerts
    previousEnableEvents = Application.EnableEvents

    On Error GoTo Failed

    Application.DisplayAlerts = False
    Application.EnableEvents = False
    Application.Visible = True

    EnsureDirectory outputRoot

    Dim rawPlan As String
    rawPlan = ReadUtf8Text(planPath)

    Dim lines() As String
    lines = Split(Replace$(rawPlan, vbCr, vbNullString), vbLf)

    If UBound(lines) < 1 Then
        Err.Raise vbObjectError + 7301, "RunRangeImageCaptureBatch", _
            "Capture plan has no data rows: " & planPath
    End If

    Dim capturesJson As String
    capturesJson = vbNullString

    Dim firstCapture As Boolean
    firstCapture = True

    Dim totalMs As Double
    totalMs = 0#

    Dim successCount As Long
    successCount = 0

    Dim currentWorkbookPath As String
    currentWorkbookPath = vbNullString

    Dim wb As Workbook
    Set wb = Nothing

    Dim i As Long
    For i = 1 To UBound(lines)
        Dim line As String
        line = lines(i)

        If Len(line) = 0 Then
            GoTo ContinueLoop
        End If

        Dim fields() As String
        fields = Split(line, vbTab)

        If UBound(fields) <> 9 Then
            Err.Raise vbObjectError + 7302, "RunRangeImageCaptureBatch", _
                "Capture-plan row " & CStr(i + 1) & _
                " has " & CStr(UBound(fields) + 1) & " fields; expected 10."
        End If

        Dim captureId As String
        Dim workbookPath As String
        Dim workbookName As String
        Dim sheetName As String
        Dim rangeAddress As String
        Dim usedRows As Long
        Dim usedColumns As Long
        Dim shapeCount As Long
        Dim relativePng As String
        Dim reason As String

        captureId = fields(0)
        workbookPath = fields(1)
        workbookName = fields(2)
        sheetName = fields(3)
        rangeAddress = fields(4)
        usedRows = CLng(fields(5))
        usedColumns = CLng(fields(6))
        shapeCount = CLng(fields(7))
        relativePng = fields(8)
        reason = fields(9)

        If StrComp(currentWorkbookPath, workbookPath, vbTextCompare) <> 0 Then
            If Not wb Is Nothing Then
                wb.Close SaveChanges:=False
                Set wb = Nothing
            End If

            Set wb = Application.Workbooks.Open( _
                Filename:=workbookPath, _
                UpdateLinks:=0, _
                ReadOnly:=True)

            currentWorkbookPath = workbookPath
        End If

        If StrComp(CStr(wb.Name), workbookName, vbTextCompare) <> 0 Then
            Err.Raise vbObjectError + 7303, "RunRangeImageCaptureBatch", _
                "Workbook name differs from capture plan. Expected " & _
                workbookName & ", opened " & CStr(wb.Name)
        End If

        Dim pngPath As String
        pngPath = outputRoot & "\" & relativePng

        Dim elapsedMs As Double
        Dim errorText As String
        Dim pngBytes As Double

        errorText = vbNullString
        elapsedMs = 0#
        pngBytes = 0#

        WriteUtf8Text resultPath & ".current.json", _
            "{""status"":""capture_started"",""workbook"":" & QuoteJson(workbookName) & _
            ",""sheet"":" & QuoteJson(sheetName) & ",""address"":" & QuoteJson(rangeAddress) & "}"
        CaptureRangeToPng _
            wb, _
            sheetName, _
            rangeAddress, _
            pngPath, _
            elapsedMs, _
            errorText

        If Len(errorText) = 0 Then
            totalMs = totalMs + elapsedMs
            successCount = successCount + 1

            If Len(Dir$(pngPath, vbNormal Or vbHidden Or vbSystem Or vbReadOnly)) > 0 Then
                pngBytes = CDbl(FileLen(pngPath))
            End If
        End If

        Dim entryJson As String
        entryJson = _
            "    {" & vbLf & _
            "      ""id"": " & QuoteJson(captureId) & "," & vbLf & _
            "      ""workbook"": " & QuoteJson(workbookName) & "," & vbLf & _
            "      ""sheet"": " & QuoteJson(sheetName) & "," & vbLf & _
            "      ""address"": " & QuoteJson(rangeAddress) & "," & vbLf & _
            "      ""relative_png"": " & QuoteJson(relativePng) & "," & vbLf & _
            "      ""used_rows"": " & CStr(usedRows) & "," & vbLf & _
            "      ""used_columns"": " & CStr(usedColumns) & "," & vbLf & _
            "      ""shape_count"": " & CStr(shapeCount) & "," & vbLf & _
            "      ""reason"": " & QuoteJson(reason) & "," & vbLf & _
            "      ""duration_ms"": " & InvariantFixed(elapsedMs, 6) & "," & vbLf & _
            "      ""png_bytes"": " & InvariantFixed(pngBytes, 0) & "," & vbLf

        If Len(errorText) = 0 Then
            entryJson = entryJson & _
                "      ""error"": null" & vbLf & _
                "    }"
        Else
            entryJson = entryJson & _
                "      ""error"": " & QuoteJson(errorText) & vbLf & _
                "    }"
        End If

        If Not firstCapture Then
            capturesJson = capturesJson & "," & vbLf
        End If

        firstCapture = False
        capturesJson = capturesJson & entryJson
        WriteUtf8Text resultPath & ".progress.json", _
            "{""status"":""running"",""captures"":[" & capturesJson & "]}"

ContinueLoop:
    Next i

    If Not wb Is Nothing Then
        wb.Close SaveChanges:=False
        Set wb = Nothing
    End If

    Dim resultJson As String
    resultJson = _
        "{" & vbLf & _
        "  ""_meta"": {" & vbLf & _
        "    ""variant"": ""vba-copypicture-chart-export""," & vbLf & _
        "    ""capture_total_ms"": " & InvariantFixed(totalMs, 6) & "," & vbLf & _
        "    ""capture_count"": " & CStr(successCount) & "," & vbLf & _
        "    ""timing_scope"": ""Range.CopyPicture plus temporary Chart.Paste and Chart.Export to PNG. Workbook open/close, capture-plan generation, and result serialization are excluded.""" & vbLf & _
        "  }," & vbLf & _
        "  ""captures"": [" & vbLf & _
        capturesJson & vbLf & _
        "  ]" & vbLf & _
        "}" & vbLf

    WriteUtf8Text resultPath, resultJson
    WriteUtf8Text resultPath & ".current.json", "{""status"":""capture_loop_finished""}"
    WriteUtf8Text resultPath & ".progress.json", _
        "{""status"":""complete"",""captures"":[" & capturesJson & "]}"

CleanExit:
    On Error Resume Next
    If Not wb Is Nothing Then wb.Close SaveChanges:=False
    Set wb = Nothing
    Application.DisplayAlerts = previousDisplayAlerts
    Application.EnableEvents = previousEnableEvents
    On Error GoTo 0
    Exit Sub

Failed:
    Dim number As Long
    Dim description As String

    number = Err.Number
    description = Err.Description

    On Error Resume Next
    If Not wb Is Nothing Then wb.Close SaveChanges:=False
    Set wb = Nothing
    Application.DisplayAlerts = previousDisplayAlerts
    Application.EnableEvents = previousEnableEvents
    On Error GoTo 0

    Err.Raise number, "RunRangeImageCaptureBatch", description
End Sub

Private Sub CaptureRangeToPng( _
    ByVal wb As Workbook, _
    ByVal sheetName As String, _
    ByVal rangeAddress As String, _
    ByVal pngPath As String, _
    ByRef elapsedMs As Double, _
    ByRef errorText As String)

    Dim ws As Worksheet
    Dim rng As Range
    Dim started As Double
    Dim attempt As Long
    Dim attemptError As String

    On Error GoTo Failed

    Set ws = wb.Worksheets(sheetName)

    If ws.Visible <> xlSheetVisible Then
        Err.Raise vbObjectError + 7310, "CaptureRangeToPng", _
            "Worksheet is not visible: " & sheetName
    End If

    wb.Activate
    ws.Activate

    Set rng = ws.Range(rangeAddress)
    rng.Select

    DoEvents

    started = QpcSeconds()
    errorText = vbNullString

    ' CopyPicture is known to be transiently flaky in automated Excel. Retry the
    ' whole clipboard/chart/export sequence, cleaning up the temporary chart after
    ' every attempt. The timer intentionally includes retry overhead because a
    ' production screenshot path must pay that cost to obtain a PNG.
    For attempt = 1 To 3
        attemptError = vbNullString

        If TryCaptureRangeToPngOnce(ws, rng, pngPath, attemptError) Then
            elapsedMs = (QpcSeconds() - started) * 1000#
            errorText = vbNullString
            GoTo CleanExit
        End If

        errorText = attemptError
        Application.CutCopyMode = False
        DoEvents
    Next attempt

    elapsedMs = (QpcSeconds() - started) * 1000#

    If Len(errorText) = 0 Then
        errorText = "CopyPicture/Chart.Export failed after 3 attempts."
    End If

CleanExit:
    On Error Resume Next
    Application.CutCopyMode = False
    Set rng = Nothing
    Set ws = Nothing
    On Error GoTo 0
    Exit Sub

Failed:
    errorText = Err.Description

    If started <> 0# Then
        elapsedMs = (QpcSeconds() - started) * 1000#
    Else
        elapsedMs = 0#
    End If

    Resume CleanExit
End Sub

Private Function TryCaptureRangeToPngOnce( _
    ByVal ws As Worksheet, _
    ByVal rng As Range, _
    ByVal pngPath As String, _
    ByRef errorText As String) As Boolean

    Dim chartObject As ChartObject
    Dim exportOk As Boolean

    On Error GoTo Failed

    On Error Resume Next
    If Len(Dir$(pngPath, vbNormal Or vbHidden Or vbSystem Or vbReadOnly)) > 0 Then
        Kill pngPath
    End If
    On Error GoTo Failed

    ' Engineering allocation budgets, not a universal CopyPicture API limit.
    Dim predictedWidth As Double
    Dim predictedHeight As Double
    predictedWidth = rng.Width * 4# / 3# + 1#
    predictedHeight = rng.Height * 4# / 3# + 1#
    If predictedWidth > 16000# Or predictedHeight > 16000# Or _
        predictedWidth * predictedHeight > 16000000# Then
        Err.Raise vbObjectError + 7319, "TryCaptureRangeToPngOnce", _
            "SAFE_CAPTURE_BUDGET: unmodified VBA baseline does not tile; range refused before CopyPicture: " & rng.Address
    End If

    rng.CopyPicture _
        Appearance:=xlScreen, _
        Format:=xlBitmap

    Set chartObject = ws.ChartObjects.Add( _
        Left:=0#, _
        Top:=0#, _
        Width:=IIf(rng.Width < 1#, 1#, rng.Width), _
        Height:=IIf(rng.Height < 1#, 1#, rng.Height))

    chartObject.Activate
    chartObject.Chart.Paste

    exportOk = chartObject.Chart.Export( _
        Filename:=pngPath, _
        FilterName:="PNG")

    If Not exportOk Then
        Err.Raise vbObjectError + 7311, "TryCaptureRangeToPngOnce", _
            "Chart.Export returned False."
    End If

    If Len(Dir$(pngPath, vbNormal Or vbHidden Or vbSystem Or vbReadOnly)) = 0 Then
        Err.Raise vbObjectError + 7312, "TryCaptureRangeToPngOnce", _
            "Chart.Export returned without creating the PNG."
    End If

    TryCaptureRangeToPngOnce = True
    GoTo CleanExit

Failed:
    errorText = Err.Description
    TryCaptureRangeToPngOnce = False

CleanExit:
    On Error Resume Next
    If Not chartObject Is Nothing Then chartObject.Delete
    Set chartObject = Nothing
    On Error GoTo 0
End Function


Private Sub EnsureDirectory(ByVal path As String)
    Dim fileSystem As Object
    Set fileSystem = CreateObject("Scripting.FileSystemObject")

    If Not fileSystem.FolderExists(path) Then
        fileSystem.CreateFolder path
    End If
End Sub

Private Function QpcSeconds() As Double
    If mFrequency = 0 Then
        If QueryPerformanceFrequency(mFrequency) = 0 Then
            Err.Raise vbObjectError + 7320, "QpcSeconds", _
                "QueryPerformanceFrequency failed."
        End If
    End If

    Dim counter As Currency

    If QueryPerformanceCounter(counter) = 0 Then
        Err.Raise vbObjectError + 7321, "QpcSeconds", _
            "QueryPerformanceCounter failed."
    End If

    QpcSeconds = CDbl(counter / mFrequency)
End Function

Private Function InvariantFixed( _
    ByVal value As Double, _
    ByVal decimals As Long) As String

    Dim pattern As String
    pattern = "0"

    If decimals > 0 Then
        pattern = pattern & "." & String$(decimals, "0")
    End If

    Dim text As String
    text = Format$(value, pattern)

    Dim decimalSeparator As String
    decimalSeparator = Application.International(xlDecimalSeparator)

    If decimalSeparator <> "." Then
        text = Replace$(text, decimalSeparator, ".")
    End If

    InvariantFixed = text
End Function

Private Function QuoteJson(ByVal value As String) As String
    QuoteJson = Chr$(34) & JsonEscape(value) & Chr$(34)
End Function

Private Function JsonEscape(ByVal value As String) As String
    value = Replace$(value, "\", "\\")
    value = Replace$(value, Chr$(34), "\" & Chr$(34))
    value = Replace$(value, vbCr, "\r")
    value = Replace$(value, vbLf, "\n")
    value = Replace$(value, vbTab, "\t")
    JsonEscape = value
End Function

Private Function ReadUtf8Text(ByVal path As String) As String
    Dim stream As Object
    Set stream = CreateObject("ADODB.Stream")

    stream.Type = 2
    stream.Charset = "utf-8"
    stream.Open
    stream.LoadFromFile path

    ReadUtf8Text = stream.ReadText

    stream.Close
End Function

Private Sub WriteUtf8Text(ByVal path As String, ByVal text As String)
    Dim stream As Object
    Set stream = CreateObject("ADODB.Stream")

    stream.Type = 2
    stream.Charset = "utf-8"
    stream.Open
    stream.WriteText text
    stream.SaveToFile path, 2
    stream.Close
End Sub
