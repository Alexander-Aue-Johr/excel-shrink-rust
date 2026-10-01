Attribute VB_Name = "LabMain"
Option Explicit

Public Function Version() As String
    Version = "excel-lab-v2"
End Function
Private Function OpenInput(ByVal path As String) As Workbook
    If LCase$(Right$(path, 5)) <> ".xlsx" Then Err.Raise 5, , "Measurement inputs must be XLSX copies"
    Application.AutomationSecurity = 3
    Set OpenInput = Application.Workbooks.Open(Filename:=path, UpdateLinks:=0, ReadOnly:=True, _
        Password:="", WriteResPassword:="", IgnoreReadOnlyRecommended:=True, Notify:=False, _
        AddToMru:=False, Local:=False, CorruptLoad:=xlNormalLoad)
End Function
Private Function Enrich(ByVal ws As Worksheet, ByVal other As Worksheet, ByVal leftSide As Boolean, ByVal options As Object, ByRef envelope As String) As String
    Dim s As String, address As String, lastRow As Long, scope As String
    Dim started As Single, variantName As String
    If leftSide Then variantName = "original" Else variantName = "shrunk"
    started = Timer
    s = LabRanges.DescribeSheet(ws): s = Left$(s, Len(s) - 1)
    envelope = ws.UsedRange.Address
    LabIO.RecordTiming "fast-used-range", ws.Name, variantName, started
    If LabIO.Flag(options, "shapes", True) Then
        started = Timer
        s = s & ",""visual_bounds"":" & LabBounds.Collect(ws, envelope)
        LabIO.RecordTiming "shape-bounds", ws.Name, variantName, started
    End If
    If Not other Is Nothing Then
        If leftSide Then address = other.UsedRange.Address Else address = ws.UsedRange.Address
        s = s & ",""matched_shrunk_range"":" & LabRanges.RangeJson(ws.Range(address))
    End If
    If LabIO.Flag(options, "hash_values") Then
        started = Timer
        s = s & ",""value_checksum"":" & LabValues.Checksum(ws, LabIO.IntAttr(options, "block_cells", 65536, 16384, 1048576), CDbl(LabIO.Attr(options, "max_cells", "20000000")))
        LabIO.RecordTiming "value-sha256", ws.Name, variantName, started
    End If
    scope = LabIO.Attr(options, "row_scope", "none")
    If scope <> "none" Then
        If scope = "all" Then
            lastRow = ws.Rows.Count
        ElseIf scope = "used-union" Then
            lastRow = ws.UsedRange.Row + ws.UsedRange.Rows.Count - 1
            If Not other Is Nothing Then lastRow = Application.Max(lastRow, other.UsedRange.Row + other.UsedRange.Rows.Count - 1)
        Else
            Err.Raise 5, , "Unknown row_scope"
        End If
        started = Timer
        s = s & ",""row_heights"":" & LabRows.Collect(ws, lastRow, LabIO.IntAttr(options, "batch_rows", 8192, 1, 65536), scope)
        LabIO.RecordTiming "row-heights", ws.Name, variantName, started
    End If
    Enrich = s & "}"
End Function
Private Sub CaptureOne(ByVal a As Worksheet, ByVal b As Worksheet, ByVal address As String, ByVal directory As String, ByVal prefix As String, _
                       ByVal appearance As Long, ByVal copyFormat As Long, ByVal storage As String, ByVal transparent As Boolean, ByVal options As Object, ByRef results As Collection)
    Dim message As String, code As Long
    On Error GoTo Failed
    LabCapture.RunPair a, b, address, directory, prefix, appearance, copyFormat, storage, transparent, _
        LabIO.Flag(options, "compare_visual_vba"), LabIO.IntAttr(options, "tile_points", 1200, 72, 6000), _
        LabIO.IntAttr(options, "max_tiles", 1024, 1, 20000), LabIO.IntAttr(options, "emf_dpi", 96, 72, 600), results
    Exit Sub
Failed:
    code = Err.Number: message = Err.Description
    If code = vbObjectError + 790 Then Err.Raise code, "LabMain.CaptureOne", message
    ' Preserve failures in the manifest. The postprocessor reports failure, not
    ' equal/empty images. Other CopyPicture variants may still be attempted.
    results.Add "{""id"":" & LabIO.J(prefix) & ",""sheet"":" & LabIO.J(a.Name) & ",""status"":""failed"",""error"":" & LabIO.J(message) & "}"
    LabIO.Progress "capture variant FAILED " & prefix & ": " & message
End Sub
Private Sub AutoCaptures(ByVal a As Worksheet, ByVal b As Worksheet, ByVal address As String, ByVal directory As String, ByVal prefix As String, ByVal options As Object, ByRef results As Collection)
    Dim appearances As Variant, formats As Variant, stores As Variant, ap As Variant, fmt As Variant, storage As Variant, root As String
    If LabIO.Flag(options, "copy_matrix") Then
        appearances = Array(xlScreen, xlPrinter): formats = Array(xlBitmap, xlPicture)
    Else
        appearances = Array(xlScreen): formats = Array(xlBitmap)
    End If
    Select Case LabIO.Attr(options, "storage", "png")
        Case "both": stores = Array("png", "bmp")
        Case "png": stores = Array("png")
        Case "bmp": stores = Array("bmp")
        Case Else: Err.Raise 5, , "Invalid capture storage"
    End Select
    For Each ap In appearances
        For Each fmt In formats
            For Each storage In stores
                root = prefix & "-a" & CStr(ap) & "-f" & CStr(fmt) & "-" & CStr(storage)
                CaptureOne a, b, address, directory, root, CLng(ap), CLng(fmt), CStr(storage), False, options, results
            Next
        Next
    Next
    If LabIO.Flag(options, "transparent_emf") Then
        ' Separate PNG-only variant, not white-pixel deletion and not BMP alpha.
        CaptureOne a, b, address, directory, prefix & "-screen-emf-transparent", xlScreen, xlPicture, "png", True, options, results
    End If
End Sub
Public Function RunBatch(ByVal requestPath As String, ByVal progressFile As String, ByVal cancelFile As String) As String
    Dim doc As Object, options As Object, pair As Object, node As Object
    Dim a As Workbook, b As Workbook, ws As Worksheet, other As Worksheet, selected As Worksheet
    Dim originals As New Collection, shrunk As New Collection, comparisons As New Collection, captures As New Collection, overflows As New Collection
    Dim oldSheets As Collection, newSheets As Collection, usedA As Object, usedB As Object
    Dim directory As String, id As String, name As String, envelope As String, address As String, entry As String
    Dim seq As Long, i As Long, code As Long, message As String, fso As Object
    Dim started As Single, batchStarted As Single
    On Error GoTo Failed
    LabIO.ProgressPath = progressFile: LabIO.CancelPath = cancelFile
    LabIO.ResetTimings: batchStarted = Timer
    Set doc = LabIO.LoadPlan(requestPath): Set options = doc.documentElement
    directory = LabIO.Attr(options, "output")
    If Len(directory) = 0 Then Err.Raise 5, , "Missing output directory"
    Set fso = CreateObject("Scripting.FileSystemObject")
    If Not fso.FolderExists(directory & "\images") Then fso.CreateFolder directory & "\images"
    If Not fso.FolderExists(directory & "\overflow") Then fso.CreateFolder directory & "\overflow"
    Application.EnableEvents = False: Application.DisplayAlerts = False: Application.AskToUpdateLinks = False
    Application.Calculation = xlCalculationManual
    Application.ScreenUpdating = True ' CopyPicture needs a real renderable window.
    LabPixels.MaxPixels = CDbl(LabIO.Attr(options, "max_pixels", "32000000"))
    For Each pair In options.SelectNodes("pairs/pair")
        seq = seq + 1: id = LabIO.Attr(pair, "id"): name = LabIO.Attr(pair, "name")
        LabIO.Progress "BEGIN workbook pair=" & id & " " & name
        Set a = OpenInput(LabIO.Attr(pair, "original"))
        Set b = Nothing
        If Len(LabIO.Attr(pair, "shrunk")) > 0 Then Set b = OpenInput(LabIO.Attr(pair, "shrunk"))
        Set oldSheets = New Collection: Set newSheets = New Collection
        Set usedA = CreateObject("Scripting.Dictionary"): Set usedB = CreateObject("Scripting.Dictionary")
        usedA.CompareMode = vbBinaryCompare: usedB.CompareMode = vbBinaryCompare
        For Each ws In a.Worksheets
            Set other = Nothing: If Not b Is Nothing Then Set other = LabIO.SheetNamed(b, ws.Name)
            LabIO.Progress "metadata original sheet=" & ws.Name
            oldSheets.Add Enrich(ws, other, True, options, envelope): usedA.Add ws.Name, envelope
            If LabIO.Flag(options, "compare_values") And Not b Is Nothing Then
                If other Is Nothing Then
                    comparisons.Add "{""file"":" & LabIO.J(name) & ",""sheet"":" & LabIO.J(ws.Name) & ",""equal"":false,""reason"":""missing-in-shrunk""}"
                Else
                    started = Timer
                    comparisons.Add "{""file"":" & LabIO.J(name) & ",""comparison"":" & LabValues.Compare(ws, other, _
                        LabIO.IntAttr(options, "block_cells", 65536, 16384, 1048576), CDbl(LabIO.Attr(options, "max_cells", "20000000")), _
                        LabIO.IntAttr(options, "max_differences", 1000, 0, 100000)) & "}"
                    LabIO.RecordTiming "value-compare", ws.Name, "pair", started
                End If
            End If
        Next
        If Not b Is Nothing Then
            For Each ws In b.Worksheets
                Set other = LabIO.SheetNamed(a, ws.Name)
                LabIO.Progress "metadata shrunk sheet=" & ws.Name
                newSheets.Add Enrich(ws, other, False, options, envelope): usedB.Add ws.Name, envelope
                If other Is Nothing And LabIO.Flag(options, "compare_values") Then comparisons.Add "{""file"":" & LabIO.J(name) & ",""sheet"":" & LabIO.J(ws.Name) & ",""equal"":false,""reason"":""added-in-shrunk""}"
            Next
        End If
        originals.Add "{""file"":" & LabIO.J(name) & ",""sheets"":" & LabIO.JoinJson(oldSheets) & "}"
        If Not b Is Nothing Then shrunk.Add "{""file"":" & LabIO.J(name) & ",""sheets"":" & LabIO.JoinJson(newSheets) & "}"
        If LabIO.Flag(options, "screenshots") Or LabIO.Flag(options, "compare_visual_vba") Or LabIO.Flag(options, "compare_visual_rust") Then
            If options.SelectNodes("captures/capture").Length = 0 Then
                i = 0
                For Each ws In a.Worksheets
                    i = i + 1: Set other = Nothing
                    address = CStr(usedA(ws.Name))
                    If Not b Is Nothing Then
                        Set other = LabIO.SheetNamed(b, ws.Name)
                        If other Is Nothing Then Err.Raise 5, , "Visual comparison: missing sheet " & ws.Name
                        address = LabRanges.AddressUnion(ws.Range(address), other.Range(CStr(usedB(ws.Name))))
                    End If
                    AutoCaptures ws, other, address, directory, id & "-s" & Format$(i, "0000"), options, captures
                Next
            Else
                i = 0
                For Each node In options.SelectNodes("captures/capture")
                    i = i + 1
                    If LabIO.Attr(node, "pair", id) = id Then
                        Set ws = LabIO.SheetNamed(a, LabIO.Attr(node, "sheet"))
                        If ws Is Nothing Then Err.Raise 5, , "Capture plan sheet not found"
                        Set other = Nothing
                        If Not b Is Nothing Then
                            Set other = LabIO.SheetNamed(b, ws.Name)
                            If other Is Nothing Then Err.Raise 5, , "Capture plan counterpart sheet missing"
                        End If
                        CaptureOne ws, other, LabIO.Attr(node, "range"), directory, id & "-p" & Format$(i, "0000"), _
                            CLng(LabIO.Attr(node, "appearance", "1")), CLng(LabIO.Attr(node, "format", "2")), LabIO.Attr(node, "storage", "png"), _
                            LabIO.Flag(node, "transparent"), options, captures
                    End If
                Next
            End If
        End If
        If Not b Is Nothing Then
            If LabIO.Flag(options, "compare_visual_vba") Or LabIO.Flag(options, "compare_visual_rust") Then
                For Each ws In b.Worksheets
                    Set other = LabIO.SheetNamed(a, ws.Name)
                    If other Is Nothing Then captures.Add "{""id"":" & LabIO.J(id & "-added-s" & CStr(ws.Index)) & _
                        ",""sheet"":" & LabIO.J(ws.Name) & ",""status"":""failed"",""error"":""Added sheet has no original visual counterpart""}"
                Next
            End If
        End If
        i = 0
        For Each node In options.SelectNodes("overflow/probe")
            i = i + 1
            If LabIO.Attr(node, "pair", id) = id Then
                Set selected = Nothing
                If LabIO.Attr(node, "variant", "original") = "original" Then
                    Set selected = LabIO.SheetNamed(a, LabIO.Attr(node, "sheet"))
                ElseIf Not b Is Nothing Then
                    Set selected = LabIO.SheetNamed(b, LabIO.Attr(node, "sheet"))
                End If
                If selected Is Nothing Then Err.Raise 5, , "Overflow sheet/variant missing"
                overflows.Add LabOverflow.Probe(selected, LabIO.Attr(node, "capture"), LabIO.Attr(node, "probe"), _
                    LabIO.Attr(node, "source"), LabIO.Attr(node, "method", "counterfactual"), directory & "\overflow", id & "-p" & Format$(i, "0000"))
            End If
        Next
        If Not b Is Nothing Then b.Close False: Set b = Nothing
        a.Close False: Set a = Nothing
        LabIO.Progress "DONE workbook pair=" & id
    Next
    If seq = 0 Then Err.Raise 5, , "No workbook pairs in request"
    LabIO.SaveText directory & "\original.json", LabIO.JoinJson(originals)
    LabIO.SaveText directory & "\shrunk.json", LabIO.JoinJson(shrunk)
    LabIO.SaveText directory & "\value-comparison.json", LabIO.JoinJson(comparisons)
    LabIO.SaveText directory & "\captures.json", "{""schema"":""excel-lab-captures-v1"",""captures"":" & LabIO.JoinJson(captures) & "}"
    LabIO.SaveText directory & "\overflow.json", LabIO.JoinJson(overflows)
    LabIO.RecordTiming "vba-batch-total", "", "", batchStarted
    LabIO.SaveTimings directory & "\timings.json"
    LabIO.SaveText directory & "\measurement-complete.json", "{""complete"":true,""pairs"":" & CStr(seq) & "}"
    LabPixels.Shutdown
    LabIO.Progress "DONE all VBA measurements"
    RunBatch = "ok": Exit Function
Failed:
    code = Err.Number: message = Err.Description
    On Error Resume Next
    If Not b Is Nothing Then b.Close False
    If Not a Is Nothing Then a.Close False
    LabPixels.Shutdown
    LabIO.SaveText directory & "\failure.json", "{""complete"":false,""error"":" & LabIO.J(message) & ",""number"":" & CStr(code) & "}"
    On Error GoTo 0
    Err.Raise code, "LabMain.RunBatch", message
End Function
