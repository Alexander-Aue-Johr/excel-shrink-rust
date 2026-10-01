Attribute VB_Name = "LabCapture"
Option Explicit

Public Function Tiles(ByVal a As Worksheet, ByVal b As Worksheet, ByVal address As String, ByVal maxPoints As Long, ByVal maxTiles As Long) As Collection
    Dim result As New Collection, area As Range, r As Long, c As Long, re As Long, ce As Long, lastR As Long, lastC As Long
    Set area = a.Range(address)
    If area.Areas.Count <> 1 Then Err.Raise 5, , "Capture plans require contiguous rectangular ranges"
    lastR = area.Row + area.Rows.Count - 1: lastC = area.Column + area.Columns.Count - 1
    r = area.Row
    Do While r <= lastR
        re = EndAt(a, b, r, lastR, maxPoints, True)
        c = area.Column
        Do While c <= lastC
            ce = EndAt(a, b, c, lastC, maxPoints, False)
            If result.Count >= maxTiles Then Err.Raise 5, "LabCapture", "Capture plan exceeds max_tiles; supply narrower ranges instead of inflated UsedRange"
            result.Add a.Range(a.Cells(r, c), a.Cells(re, ce)).Address
            c = ce + 1
        Loop
        r = re + 1
    Loop
    Set Tiles = result
End Function
Private Function EndAt(ByVal a As Worksheet, ByVal b As Worksheet, ByVal first As Long, ByVal last As Long, ByVal maxPoints As Double, ByVal rows As Boolean) As Long
    Dim lo As Long, hi As Long, mid As Long, sizeA As Double, sizeB As Double, ra As Range, rb As Range
    lo = first: hi = last
    Do While lo < hi
        mid = lo + (hi - lo + 1) \ 2
        If rows Then
            Set ra = a.Range(a.Cells(first, 1), a.Cells(mid, 1))
            sizeA = ra.Height
            sizeB = sizeA
            If Not b Is Nothing Then sizeB = b.Range(b.Cells(first, 1), b.Cells(mid, 1)).Height
        Else
            Set ra = a.Range(a.Cells(1, first), a.Cells(1, mid))
            sizeA = ra.Width
            sizeB = sizeA
            If Not b Is Nothing Then sizeB = b.Range(b.Cells(1, first), b.Cells(1, mid)).Width
        End If
        If Application.Max(sizeA, sizeB) <= maxPoints Then lo = mid Else hi = mid - 1
    Loop
    EndAt = lo
End Function
Public Sub RunPair(ByVal a As Worksheet, ByVal b As Worksheet, ByVal address As String, ByVal directory As String, ByVal prefix As String, _
                   ByVal appearance As Long, ByVal copyFormat As Long, ByVal storage As String, ByVal transparent As Boolean, _
                   ByVal compareVba As Boolean, ByVal maxPoints As Long, ByVal maxTiles As Long, ByVal dpi As Long, ByRef results As Collection)
    Dim plan As Collection, item As Variant, tile As Long, imageA As LongPtr, imageB As LongPtr
    Dim pathA As String, pathB As String, relativeA As String, relativeB As String, details As String
    Dim w As Long, h As Long, code As Long, message As String, started As Single
    Dim rangeA As Range, rangeB As Range, sequenceA As Long, sequenceB As Long
    On Error GoTo Failed
    Set plan = Tiles(a, b, address, maxPoints, maxTiles)
    For Each item In plan
        tile = tile + 1: started = Timer
        Set rangeA = a.Range(CStr(item))
        Set rangeB = Nothing: If Not b Is Nothing Then Set rangeB = b.Range(CStr(item))
        LabIO.Progress "capture " & prefix & " sheet=" & a.Name & " tile=" & CStr(tile) & "/" & CStr(plan.Count) & " " & CStr(item)
        relativeA = "images/" & prefix & "-t" & Format$(tile, "00000") & "-original." & storage
        relativeB = "images/" & prefix & "-t" & Format$(tile, "00000") & "-shrunk." & storage
        pathA = directory & "\" & Replace$(relativeA, "/", "\")
        pathB = directory & "\" & Replace$(relativeB, "/", "\")
        If rangeA.Width <= 0 Or rangeA.Height <= 0 Then Err.Raise 5, , "Original tile has no visible dimensions"
        imageA = LabPixels.Capture(rangeA, appearance, copyFormat, transparent, dpi)
        sequenceA = LabPixels.LastSequence
        LabPixels.Save imageA, pathA, storage
        details = "{""id"":" & LabIO.J(prefix & "-t" & Format$(tile, "00000")) & ",""sheet"":" & LabIO.J(a.Name) & _
            ",""requested_range"":" & LabIO.J(address) & ",""tile_range"":" & LabIO.J(CStr(item)) & _
            ",""appearance"":" & CStr(appearance) & ",""copy_format"":" & CStr(copyFormat) & _
            ",""storage"":" & LabIO.J(storage) & ",""transparent_canvas_requested"":" & LabIO.B(transparent) & _
            ",""bmp_background_policy"":""RGB24-white"",""capture_method"":""Range.CopyPicture; not a desktop screenshot"",""full_painted_extent_proven"":false,""original"":" & LabIO.J(relativeA)
        If Not b Is Nothing Then
            imageB = LabPixels.Capture(rangeB, appearance, copyFormat, transparent, dpi)
            sequenceB = LabPixels.LastSequence
            LabPixels.Save imageB, pathB, storage
            LabPixels.Size imageB, w, h
            details = details & ",""shrunk"":" & LabIO.J(relativeB) & ",""shrunk_width"":" & CStr(w) & ",""shrunk_height"":" & CStr(h)
            If compareVba Then details = details & ",""vba_visual_compare"":" & LabPixels.Compare(imageA, imageB)
        Else
            details = details & ",""shrunk"":null"
        End If
        LabPixels.Size imageA, w, h
        details = details & ",""original_width"":" & CStr(w) & ",""original_height"":" & CStr(h) & _
            ",""original_clipboard_sequence"":" & CStr(sequenceA) & ",""shrunk_clipboard_sequence"":" & CStr(sequenceB) & _
            ",""capture_seconds"":" & LabIO.N(LabIO.SecondsSince(started)) & "}"
        results.Add details
        LabPixels.Dispose imageA: LabPixels.Dispose imageB
        LabIO.Progress "capture done " & prefix & " tile=" & CStr(tile)
    Next
    Exit Sub
Failed:
    code = Err.Number: message = Err.Description
    LabPixels.Dispose imageA: LabPixels.Dispose imageB
    Err.Raise code, "LabCapture.RunPair", message
End Sub
