Attribute VB_Name = "DrawingBounds"
Option Explicit

#If VBA7 Then
Private Declare PtrSafe Function QueryPerformanceCounter Lib "kernel32" (ByRef value As Currency) As Long
Private Declare PtrSafe Function QueryPerformanceFrequency Lib "kernel32" (ByRef value As Currency) As Long
#Else
Private Declare Function QueryPerformanceCounter Lib "kernel32" (ByRef value As Currency) As Long
Private Declare Function QueryPerformanceFrequency Lib "kernel32" (ByRef value As Currency) As Long
#End If

Private mFrequency As Currency

Public Sub RunDrawingBoundsBatch(ByVal fileListPath As String, ByVal outputPath As String)
    Dim previousDisplayAlerts As Boolean
    Dim previousEnableEvents As Boolean

    previousDisplayAlerts = Application.DisplayAlerts
    previousEnableEvents = Application.EnableEvents

    On Error GoTo Failed

    Application.DisplayAlerts = False
    Application.EnableEvents = False

    Dim rawList As String
    rawList = ReadUtf8Text(fileListPath)

    Dim lines() As String
    lines = Split(Replace$(rawList, vbCr, vbNullString), vbLf)

    Dim seen As Object
    Set seen = CreateObject("Scripting.Dictionary")
    seen.CompareMode = vbTextCompare

    Dim json As String
    json = "{" & vbLf

    Dim firstWorkbook As Boolean
    firstWorkbook = True

    Dim i As Long
    For i = LBound(lines) To UBound(lines)
        Dim path As String
        path = Trim$(lines(i))
        If Len(path) > 0 Then
            Dim workbookName As String
            Dim workbookJson As String
            workbookJson = MeasureWorkbook(path, workbookName)

            If seen.Exists(workbookName) Then
                Err.Raise vbObjectError + 7101, "RunDrawingBoundsBatch", _
                    "Duplicate workbook filename in corpus: " & workbookName
            End If
            seen.Add workbookName, True

            If Not firstWorkbook Then json = json & "," & vbLf
            firstWorkbook = False
            json = json & "  " & QuoteJson(workbookName) & ": " & workbookJson
        End If
    Next i

    json = json & vbLf & "}" & vbLf
    WriteUtf8Text outputPath, json

CleanExit:
    Application.DisplayAlerts = previousDisplayAlerts
    Application.EnableEvents = previousEnableEvents
    Exit Sub

Failed:
    Dim message As String
    message = "Drawing bounds VBA experiment failed: " & Err.Description
    On Error Resume Next
    Application.DisplayAlerts = previousDisplayAlerts
    Application.EnableEvents = previousEnableEvents
    On Error GoTo 0
    Err.Raise vbObjectError + 7100, "RunDrawingBoundsBatch", message
End Sub

Private Function MeasureWorkbook(ByVal path As String, ByRef workbookName As String) As String
    Dim wb As Workbook
    On Error GoTo Failed

    Set wb = Application.Workbooks.Open( _
        Filename:=path, _
        UpdateLinks:=0, _
        ReadOnly:=True)

    workbookName = wb.Name

    Dim json As String
    json = "{" & vbLf

    Dim firstSheet As Boolean
    firstSheet = True

    Dim ws As Worksheet
    For Each ws In wb.Worksheets
        Dim started As Double
        Dim bounds As Variant

        started = QpcSeconds()
        bounds = VisibleDrawingBounds(ws)
        Dim elapsedMs As Double
        elapsedMs = (QpcSeconds() - started) * 1000#

        If Not firstSheet Then json = json & "," & vbLf
        firstSheet = False

        json = json & _
            "    " & QuoteJson(ws.Name) & ": {" & vbLf & _
            "      ""right"": " & InvariantFixed(CDbl(bounds(0)), 6) & "," & vbLf & _
            "      ""bottom"": " & InvariantFixed(CDbl(bounds(1)), 6) & "," & vbLf & _
            "      ""duration_ms"": " & InvariantFixed(elapsedMs, 6) & vbLf & _
            "    }"
    Next ws

    json = json & vbLf & "  }"

    wb.Close SaveChanges:=False
    Set wb = Nothing

    MeasureWorkbook = json
    Exit Function

Failed:
    Dim number As Long
    Dim description As String
    number = Err.Number
    description = Err.Description

    On Error Resume Next
    If Not wb Is Nothing Then wb.Close SaveChanges:=False
    Set wb = Nothing
    On Error GoTo 0

    Err.Raise number, "MeasureWorkbook(" & path & ")", description
End Function

Private Function VisibleDrawingBounds(ByVal ws As Worksheet) As Variant
    Dim maxRight As Double
    Dim maxBottom As Double
    maxRight = 0#
    maxBottom = 0#

    Dim shp As Shape
    For Each shp In ws.Shapes
        If shp.Visible <> msoFalse Then
            Dim leftValue As Double
            Dim topValue As Double
            Dim widthValue As Double
            Dim heightValue As Double
            Dim angle As Double
            Dim rotatedWidth As Double
            Dim rotatedHeight As Double
            Dim rightValue As Double
            Dim bottomValue As Double

            leftValue = CDbl(shp.Left)
            topValue = CDbl(shp.Top)
            widthValue = CDbl(shp.Width)
            heightValue = CDbl(shp.Height)
            angle = CDbl(shp.Rotation) * 3.14159265358979# / 180#

            rotatedWidth = Abs(widthValue * Cos(angle)) + Abs(heightValue * Sin(angle))
            rotatedHeight = Abs(widthValue * Sin(angle)) + Abs(heightValue * Cos(angle))

            rightValue = leftValue + widthValue / 2# + rotatedWidth / 2#
            bottomValue = topValue + heightValue / 2# + rotatedHeight / 2#

            If rightValue > maxRight Then maxRight = rightValue
            If bottomValue > maxBottom Then maxBottom = bottomValue
        End If
    Next shp

    VisibleDrawingBounds = Array(maxRight, maxBottom)
End Function

Private Function QpcSeconds() As Double
    If mFrequency = 0 Then
        If QueryPerformanceFrequency(mFrequency) = 0 Then
            Err.Raise vbObjectError + 7102, "QpcSeconds", "QueryPerformanceFrequency failed."
        End If
    End If

    Dim counter As Currency
    If QueryPerformanceCounter(counter) = 0 Then
        Err.Raise vbObjectError + 7103, "QpcSeconds", "QueryPerformanceCounter failed."
    End If

    QpcSeconds = CDbl(counter / mFrequency)
End Function

Private Function InvariantFixed(ByVal value As Double, ByVal decimals As Long) As String
    Dim pattern As String
    pattern = "0"
    If decimals > 0 Then pattern = pattern & "." & String$(decimals, "0")

    Dim text As String
    text = Format$(value, pattern)

    Dim decimalSeparator As String
    decimalSeparator = Application.International(xlDecimalSeparator)
    If decimalSeparator <> "." Then text = Replace$(text, decimalSeparator, ".")

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
