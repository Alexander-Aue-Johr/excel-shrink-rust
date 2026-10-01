Attribute VB_Name = "LabOverflow"
Option Explicit

' EXPERIMENT ONLY. This does not replace the repository's production overflow
' algorithm. It never saves a workbook. Inputs must be disposable local copies.
Public Function Probe(ByVal ws As Worksheet, ByVal captureAddress As String, ByVal probeAddress As String, ByVal sourceAddress As String, ByVal method As String, ByVal directory As String, ByVal id As String) As String
    Dim rg As Range, strip As Range, source As Range, overlap As Range, value As Variant
    Dim scratch As Workbook
    Dim first As LongPtr, repeat As LongPtr, withoutText As LongPtr, p1 As LongPtr, p2 As LongPtr
    Dim w As Long, h As Long, x As Long, y As Long, width As Long, height As Long
    Dim saved As Boolean, code As Long, message As String, beforeHeight As Double, beforeWidth As Double
    Dim result As String, sheetName As String
    On Error GoTo Failed
    sheetName = ws.Name
    If method = "counterfactual" Then
        ' A new disposable sheet copy isolates rich-text formatting and repeated
        ' experiments. Assigning Value2 alone would not restore rich-text runs.
        ws.Copy
        Set scratch = Application.ActiveWorkbook
        Set ws = scratch.Worksheets(1)
    End If
    Set rg = ws.Range(captureAddress): Set strip = ws.Range(probeAddress)
    If rg.Areas.Count <> 1 Or strip.Areas.Count <> 1 Then Err.Raise 5, , "Overflow ranges must be rectangles"
    Set overlap = Application.Intersect(rg, strip)
    If overlap Is Nothing Then Err.Raise 5, , "Probe must lie inside capture"
    If overlap.CountLarge <> strip.CountLarge Then Err.Raise 5, , "Probe only partly inside capture"
    beforeHeight = rg.Height: beforeWidth = rg.Width
    If beforeHeight <= 0 Or beforeWidth <= 0 Or beforeHeight > 6000 Or beforeWidth > 6000 Then Err.Raise 5, , "Unsafe overflow capture geometry"
    LabIO.Progress "overflow experiment " & id & " sheet=" & ws.Name & " mode=" & method
    first = LabPixels.Capture(rg, xlScreen, xlBitmap, False)
    repeat = LabPixels.Capture(rg, xlScreen, xlBitmap, False)
    If LabPixels.PixelHash(first) <> LabPixels.PixelHash(repeat) Then
        result = "{""status"":""inconclusive-render-unstable""}": GoTo Done
    End If
    LabPixels.Dispose repeat
    LabPixels.Size first, w, h
    x = CLng(Fix((strip.Left - rg.Left) / beforeWidth * w))
    y = CLng(Fix((strip.Top - rg.Top) / beforeHeight * h))
    width = CLng(Fix(strip.Width / beforeWidth * w))
    height = CLng(Fix(strip.Height / beforeHeight * h))
    width = Application.Min(width, w - x): height = Application.Min(height, h - y)
    p1 = LabPixels.Crop(first, x, y, width, height)
    LabPixels.Save p1, directory & "\" & id & "-before.png", "png"
    If method = "nonuniform" Then
        result = "{""status"":""experimental"",""nonuniform_pixels"":" & LabIO.B(LabPixels.NonUniform(p1)) & _
            ",""interpretation"":""Pixel variation relative to first interior pixel; not proof of text overflow or emptiness""}"
        GoTo Done
    End If
    If method <> "counterfactual" Then Err.Raise 5, , "Unknown overflow method"
    Set source = ws.Range(sourceAddress)
    If source.CountLarge <> 1 Then Err.Raise 5, , "Counterfactual requires one constant text cell"
    If source.HasFormula Or source.MergeCells Or ws.ProtectContents Then Err.Raise 5, , "Formula/merged/protected source not supported by this experiment"
    value = source.Value2
    If VarType(value) <> vbString Or Len(CStr(value)) = 0 Then Err.Raise 5, , "Source must contain constant nonempty text"
    Set overlap = Application.Intersect(source, rg)
    If overlap Is Nothing Then Err.Raise 5, , "Source must be in capture (otherwise CopyPicture can omit incoming overflow)"
    Set overlap = Application.Intersect(source, strip)
    If Not overlap Is Nothing Then Err.Raise 5, , "Probe must be outside the source cell"
    saved = True: source.ClearContents
    withoutText = LabPixels.Capture(rg, xlScreen, xlBitmap, False)
    If rg.Height <> beforeHeight Or rg.Width <> beforeWidth Then
        result = "{""status"":""inconclusive-geometry-changed""}": GoTo Done
    End If
    p2 = LabPixels.Crop(withoutText, x, y, width, height)
    LabPixels.Save p2, directory & "\" & id & "-without-source.png", "png"
    result = "{""status"":""experimental"",""pixel_compare"":" & LabPixels.Compare(p1, p2, 14) & _
        ",""interpretation"":""Change outside a removed constant-text source; conditional formatting and paint effects can also contribute. Crop mapping is point-to-pixel proportional, not an exact cell-pixel oracle.""}"
Done:
    If saved Then source.Value2 = value: saved = False
    LabPixels.Dispose first: LabPixels.Dispose repeat: LabPixels.Dispose withoutText
    LabPixels.Dispose p1: LabPixels.Dispose p2
    If Not scratch Is Nothing Then scratch.Close False: Set scratch = Nothing
    Probe = "{""id"":" & LabIO.J(id) & ",""sheet"":" & LabIO.J(sheetName) & ",""capture"":" & LabIO.J(captureAddress) & _
        ",""probe"":" & LabIO.J(probeAddress) & ",""method"":" & LabIO.J(method) & ",""result"":" & result & "}"
    Exit Function
Failed:
    code = Err.Number: message = Err.Description
    On Error Resume Next
    If saved Then source.Value2 = value
    LabPixels.Dispose first: LabPixels.Dispose repeat: LabPixels.Dispose withoutText
    LabPixels.Dispose p1: LabPixels.Dispose p2
    If Not scratch Is Nothing Then scratch.Close False
    On Error GoTo 0
    Err.Raise code, "LabOverflow.Probe", message
End Function
