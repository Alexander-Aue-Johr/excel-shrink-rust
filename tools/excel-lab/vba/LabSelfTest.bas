Attribute VB_Name = "LabSelfTest"
Option Explicit

Private Sub Assert(ByVal condition As Boolean, ByVal label As String)
    If Not condition Then Err.Raise vbObjectError + 850, "LabSelfTest", label
End Sub
Public Function Run(Optional ByVal includePictures As Boolean = False) As String
    Dim a As Workbook, b As Workbook, wa As Worksheet, wb As Worksheet, sh As Shape
    Dim ha As String, hb As String, env As String, s As String
    Dim i1 As LongPtr, i2 As LongPtr, cropped As LongPtr, code As Long, message As String
    On Error GoTo Failed
    LabHash.SelfTest
    Set a = Application.Workbooks.Add(xlWBATWorksheet): Set wa = a.Worksheets(1)
    Set b = Application.Workbooks.Add(xlWBATWorksheet): Set wb = b.Worksheets(1)
    wa.Range("B2").Value2 = 1.25: wa.Range("C3").Value2 = "Gr" & ChrW$(252) & "n"
    wa.Range("A5").Value2 = CVErr(xlErrNA): wa.Range("B6").Value2 = True
    wb.Range("B2").Value2 = 1.25: wb.Range("C3").Value2 = wa.Range("C3").Value2
    wb.Range("A5").Value2 = CVErr(xlErrNA): wb.Range("B6").Value2 = True
    ha = LabValues.Checksum(wa, 16384, 1000000): hb = LabValues.Checksum(wb, 32768, 1000000)
    Assert InStr(ha, "4af8b3b48e57b3fcbc72881922754b1cd3e37e366ddbc51f31cab80c8e4508ce") > 0, "Independent XLVALUES1 typed-coordinate fixture digest"
    Assert ha = hb, "Equal values must have equal hashes across batch sizes"
    wb.Range("Z1000").Interior.Color = RGB(220, 220, 220)
    hb = LabValues.Checksum(wb, 65536, 1000000)
    Assert ha = hb, "Formatting-only UsedRange padding must not change value hash"
    s = LabValues.Compare(wa, wb, 16384, 1000000, 2)
    Assert InStr(s, """difference_count"":0") > 0, "Padded pair must compare equal"
    wb.Range("B2").NumberFormat = "@": wb.Range("B2").Value2 = "1.25"
    Assert ha <> LabValues.Checksum(wb, 65536, 1000000), "Type changes must alter hash"
    s = LabValues.Compare(wa, wb, 16384, 1000000, 0)
    Assert InStr(s, """difference_count"":1") > 0 And InStr(s, """details_truncated"":true") > 0, "Count/truncation semantics"
    Set sh = wa.Shapes.AddShape(1, 1200, 800, 100, 70)
    s = LabBounds.Collect(wa, env)
    Assert InStr(s, """painted_pixel_bounds_proven"":false") > 0, "Bounds must not overclaim paint coverage"
    Assert wa.Range(env).Width >= 1300, "Shape outside UsedRange must expand envelope"
    Assert JsonDiffProbe.SelfTest() = "ok", "Existing row probe"
    wa.Rows("1:10").RowHeight = 12
    s = LabRows.Collect(wa, 10, 3, "used-union")
    Assert InStr(s, """from"":1,""to"":10") > 0, "Merge row runs across VBA batch boundaries"
    i1 = LabPixels.Solid(8, 8, &HFFFFFFFF): i2 = LabPixels.Solid(8, 8, &HFFFF0000)
    Assert InStr(LabPixels.Compare(i1, i1), """equal"":true") > 0, "Owned bitmap identity"
    Assert InStr(LabPixels.Compare(i1, i2), """changed_pixels"":64") > 0, "Exact pixel difference count"
    cropped = LabPixels.Crop(i1, 1, 1, 4, 4)
    Assert Not LabPixels.NonUniform(cropped), "Uniform crop"
    LabPixels.Dispose i1: LabPixels.Dispose i2: LabPixels.Dispose cropped
    If includePictures Then
        wa.Range("A1").Value2 = "Clipboard capture self-test"
        i1 = LabPixels.Capture(wa.Range("A1:E4"), xlScreen, xlBitmap, False)
        i2 = LabPixels.Capture(wa.Range("A1:E4"), xlScreen, xlBitmap, False)
        Assert LabPixels.PixelHash(i1) = LabPixels.PixelHash(i2), "Repeat capture stability; desktop/clipboard interference?"
        LabPixels.Dispose i1: LabPixels.Dispose i2
        i1 = LabPixels.Capture(wa.Range("A1:E4"), xlScreen, xlPicture, True)
        Assert Len(LabPixels.PixelHash(i1)) = 64, "EMF transparent-canvas route"
        LabPixels.Dispose i1
    End If
    b.Close False: Set b = Nothing: a.Close False: Set a = Nothing
    LabPixels.Shutdown: Run = "ok": Exit Function
Failed:
    code = Err.Number: message = Err.Description
    On Error Resume Next
    LabPixels.Dispose i1: LabPixels.Dispose i2: LabPixels.Dispose cropped
    If Not b Is Nothing Then b.Close False
    If Not a Is Nothing Then a.Close False
    LabPixels.Shutdown
    On Error GoTo 0
    Err.Raise code, "LabSelfTest.Run", message
End Function
