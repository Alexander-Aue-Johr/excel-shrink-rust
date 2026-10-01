Attribute VB_Name = "LabValues"
Option Explicit

Private Type Buffer
    Bytes() As Byte
    Used As Long
End Type
Private Sub Flush(ByRef state As LabHash.State, ByRef buffer As Buffer)
    If buffer.Used > 0 Then LabHash.Feed state, VarPtr(buffer.Bytes(0)), buffer.Used
    buffer.Used = 0
End Sub
Private Sub AppendBytes(ByRef state As LabHash.State, ByRef buffer As Buffer, ByVal pointer As LongPtr, ByVal length As Long)
    Dim count As Long
    Do While length > 0
        count = UBound(buffer.Bytes) + 1 - buffer.Used
        If length < count Then count = length
        LabHash.CopyBytes buffer.Bytes(buffer.Used), ByVal pointer, count
        buffer.Used = buffer.Used + count: length = length - count: pointer = pointer + count
        If buffer.Used = UBound(buffer.Bytes) + 1 Then Flush state, buffer
    Loop
End Sub
Private Sub PutLong(ByRef state As LabHash.State, ByRef buffer As Buffer, ByVal value As Long)
    AppendBytes state, buffer, VarPtr(value), 4
End Sub
Private Sub PutValue(ByRef state As LabHash.State, ByRef buffer As Buffer, ByRef value As Variant)
    Dim tag As Byte, num As Double, text As String, code As Long
    Select Case VarType(value)
        Case vbEmpty: tag = 0
        Case vbNull: tag = 1
        Case vbDouble, vbSingle, vbInteger, vbLong, vbCurrency, vbDecimal, vbByte, vbDate: tag = 2
        Case vbString: tag = 3
        Case vbBoolean: tag = 4
        Case vbError: tag = 5
        Case Else: Err.Raise 5, "LabValues", "Unsupported Value2 type " & CStr(VarType(value))
    End Select
    AppendBytes state, buffer, VarPtr(tag), 1
    Select Case tag
        Case 2
            num = CDbl(value): If num = 0 Then num = 0#
            AppendBytes state, buffer, VarPtr(num), 8
        Case 3
            text = CStr(value): PutLong state, buffer, LenB(text)
            If LenB(text) > 0 Then AppendBytes state, buffer, StrPtr(text), LenB(text)
        Case 4
            tag = 0: If CBool(value) Then tag = 1
            AppendBytes state, buffer, VarPtr(tag), 1
        Case 5
            code = LabHash.ErrorCode(value): PutLong state, buffer, code
    End Select
End Sub
Public Function At(ByRef values As Variant, ByVal row As Long, ByVal column As Long) As Variant
    If IsArray(values) Then At = values(row, column) Else At = values
End Function
Public Function Same(ByRef a As Variant, ByRef b As Variant) As Boolean
    ' .Value2 numbers are Doubles; no formatting/date/currency conversions.
    If VarType(a) <> VarType(b) Then Exit Function
    Select Case VarType(a)
        Case vbEmpty, vbNull: Same = True
        Case vbError: Same = (LabHash.ErrorCode(a) = LabHash.ErrorCode(b))
        Case vbString: Same = (StrComp(CStr(a), CStr(b), vbBinaryCompare) = 0)
        Case Else: Same = (a = b)
    End Select
End Function
Public Function TypedJson(ByRef value As Variant) As String
    Dim kind As String, body As String
    Select Case VarType(value)
        Case vbEmpty: kind = "empty": body = "null"
        Case vbNull: kind = "null": body = "null"
        Case vbError: kind = "error": body = CStr(LabHash.ErrorCode(value))
        Case vbBoolean: kind = "boolean": body = LabIO.B(CBool(value))
        Case vbString: kind = "string": body = LabIO.J(CStr(value))
        Case Else: kind = "number": body = LabIO.N(CDbl(value))
    End Select
    TypedJson = "{""type"":" & LabIO.J(kind) & ",""value"":" & body
    If kind = "number" Then TypedJson = TypedJson & ",""ieee754_le"":" & LabIO.J(LabHash.NumberBits(CDbl(value)))
    TypedJson = TypedJson & "}"
End Function
Public Function Checksum(ByVal ws As Worksheet, ByVal blockCells As Long, ByVal maxCells As Double) As String
    Dim state As LabHash.State, buffer As Buffer, used As Range, part As Range
    Dim values As Variant, value As Variant, row As Long, col As Long, startRow As Long, rows As Long, count As Long
    Dim nonempty As Double, digest As String, magic As String, code As Long, message As String
    Dim columns As Long, absoluteRow As Long, absoluteColumn As Long
    On Error GoTo Failed
    Set used = ws.UsedRange
    If used.CountLarge > maxCells Then Err.Raise 5, , "Value hash scan exceeds max_cells; increase explicitly. Sheet: " & ws.Name
    ReDim buffer.Bytes(0 To 65535)
    LabHash.Start state
    magic = "XLVALUES1" ' UTF-16LE domain tag; fixed 18 bytes
    AppendBytes state, buffer, StrPtr(magic), LenB(magic)
    columns = used.Columns.Count
    rows = Application.Max(1, blockCells \ columns)
    For startRow = 1 To used.Rows.Count Step rows
        count = Application.Min(rows, used.Rows.Count - startRow + 1)
        Set part = used.Rows(startRow).Resize(count, used.Columns.Count)
        values = part.Value2
        absoluteRow = part.Row: absoluteColumn = part.Column
        For row = 1 To count
            For col = 1 To columns
                value = At(values, row, col)
                ' Empty padding does not affect the digest. Coordinates of every
                ' nonempty value DO; "" and Empty remain distinct.
                If Not IsEmpty(value) Then
                    PutLong state, buffer, absoluteRow + row - 1
                    PutLong state, buffer, absoluteColumn + col - 1
                    PutValue state, buffer, value: nonempty = nonempty + 1
                End If
            Next
        Next
        LabIO.Progress "values hash sheet=" & ws.Name & " through row=" & CStr(part.Row + count - 1)
    Next
    Flush state, buffer: digest = LabHash.Finish(state)
    Checksum = "{""algorithm"":""sha256-xlvalues1"",""semantics"":""Value2; nonempty coordinates+types; UTF16LE strings; IEEE754LE numbers; no formula text or formatting"",""nonempty_cells"":" & LabIO.N(nonempty) & ",""sha256"":" & LabIO.J(digest) & "}"
    Exit Function
Failed:
    code = Err.Number: message = Err.Description: LabHash.Dispose state
    Err.Raise code, "LabValues.Checksum", message
End Function
Public Function Compare(ByVal a As Worksheet, ByVal b As Worksheet, ByVal blockCells As Long, ByVal maxCells As Double, ByVal limit As Long) As String
    Dim address As String, areaA As Range, areaB As Range, chunkA As Range, chunkB As Range
    Dim av As Variant, bv As Variant, va As Variant, vb As Variant
    Dim first As Long, count As Long, rows As Long, r As Long, c As Long
    Dim columns As Long, absoluteRow As Long, absoluteColumn As Long
    Dim differences As Double, details As New Collection
    address = LabRanges.AddressUnion(a.UsedRange, b.UsedRange)
    Set areaA = a.Range(address): Set areaB = b.Range(address)
    If areaA.CountLarge > maxCells Then Err.Raise 5, "LabValues.Compare", "Union scan exceeds max_cells: " & a.Name
    columns = areaA.Columns.Count
    rows = Application.Max(1, blockCells \ columns)
    For first = 1 To areaA.Rows.Count Step rows
        count = Application.Min(rows, areaA.Rows.Count - first + 1)
        Set chunkA = areaA.Rows(first).Resize(count, areaA.Columns.Count)
        Set chunkB = areaB.Rows(first).Resize(count, areaB.Columns.Count)
        av = chunkA.Value2: bv = chunkB.Value2
        absoluteRow = chunkA.Row: absoluteColumn = chunkA.Column
        For r = 1 To count
            For c = 1 To columns
                va = At(av, r, c): vb = At(bv, r, c)
                If Not Same(va, vb) Then
                    differences = differences + 1
                    If details.Count < limit Then details.Add "{""row"":" & CStr(absoluteRow + r - 1) & _
                        ",""column"":" & CStr(absoluteColumn + c - 1) & ",""original"":" & TypedJson(va) & ",""shrunk"":" & TypedJson(vb) & "}"
                End If
            Next
        Next
        LabIO.Progress "values compare sheet=" & a.Name & " through row=" & CStr(chunkA.Row + count - 1)
    Next
    Compare = "{""sheet"":" & LabIO.J(a.Name) & ",""range"":" & LabIO.J(address) & _
        ",""equal"":" & LabIO.B(differences = 0) & ",""difference_count"":" & LabIO.N(differences) & _
        ",""details_truncated"":" & LabIO.B(differences > details.Count) & ",""differences"":" & LabIO.JoinJson(details) & "}"
End Function
