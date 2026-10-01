Attribute VB_Name = "LabIO"
Option Explicit

Public ProgressPath As String
Public CancelPath As String
Private mTimings As Collection

Public Function J(ByVal s As String) As String
    Dim i As Long, u As Long, parts() As String
    ReDim parts(0 To Len(s) + 1): parts(0) = Chr$(34)
    For i = 1 To Len(s)
        u = AscW(Mid$(s, i, 1)) And &HFFFF&
        Select Case u
            Case 34, 92: parts(i) = "\" & Chr$(u)
            Case 32 To 126: parts(i) = Chr$(u)
            Case Else: parts(i) = "\u" & Right$("0000" & Hex$(u), 4)
        End Select
    Next
    parts(UBound(parts)) = Chr$(34): J = Join(parts, "")
End Function
Public Function N(ByVal d As Double) As String
    N = Trim$(Str$(d))
    If Left$(N, 1) = "." Then N = "0" & N
    If Left$(N, 2) = "-." Then N = "-0" & Mid$(N, 2)
End Function
Public Function B(ByVal flag As Boolean) As String
    If flag Then B = "true" Else B = "false"
End Function
Public Function JoinJson(ByVal entries As Collection) As String
    Dim parts() As String, i As Long
    If entries.Count = 0 Then JoinJson = "[]": Exit Function
    ReDim parts(0 To entries.Count - 1)
    For i = 1 To entries.Count: parts(i - 1) = CStr(entries(i)): Next
    JoinJson = "[" & Join(parts, ",") & "]"
End Function
Public Function Attr(ByVal node As Object, ByVal key As String, Optional ByVal fallback As String = "") As String
    Dim item As Object
    Set item = node.Attributes.getNamedItem(key)
    If item Is Nothing Then Attr = fallback Else Attr = CStr(item.Text)
End Function
Public Function Flag(ByVal node As Object, ByVal key As String, Optional ByVal fallback As Boolean = False) As Boolean
    Dim s As String: s = Attr(node, key, LCase$(CStr(fallback)))
    If s <> "true" And s <> "false" Then Err.Raise 5, "LabIO", "Invalid boolean: " & key
    Flag = (s = "true")
End Function
Public Function IntAttr(ByVal node As Object, ByVal key As String, ByVal fallback As Long, ByVal lo As Long, ByVal hi As Long) As Long
    Dim s As String, d As Double
    s = Attr(node, key, CStr(fallback))
    If s Like "*[!0-9]*" Or Len(s) = 0 Then Err.Raise 5, "LabIO", "Invalid integer: " & key
    d = CDbl(s)
    If d < lo Or d > hi Then Err.Raise 5, "LabIO", "Out of range: " & key
    IntAttr = CLng(d)
End Function
Public Sub Progress(ByVal message As String)
    Dim fso As Object, writer As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If Len(CancelPath) > 0 Then
        If fso.FileExists(CancelPath) Then Err.Raise vbObjectError + 790, "LabIO", "Cancelled by caller"
    End If
    If Len(ProgressPath) = 0 Then Exit Sub
    Set writer = fso.OpenTextFile(ProgressPath, 8, True, 0)
    ' JSON escaping keeps the progress file ASCII even on non-English Windows.
    writer.WriteLine Format$(Now, "hh:nn:ss") & " " & J(message)
    writer.Close
End Sub
Public Sub SaveText(ByVal path As String, ByVal text As String)
    Dim stream As Object, fso As Object
    Set fso = CreateObject("Scripting.FileSystemObject")
    If fso.FileExists(path) Or fso.FileExists(path & ".partial") Then Err.Raise 58, "LabIO", "Output already exists: " & path
    Set stream = CreateObject("ADODB.Stream")
    stream.Type = 2: stream.Charset = "utf-8": stream.Open
    stream.WriteText text: stream.SaveToFile path & ".partial", 1: stream.Close
    fso.MoveFile path & ".partial", path
End Sub
Public Function LoadPlan(ByVal path As String) As Object
    Dim doc As Object: Set doc = CreateObject("MSXML2.DOMDocument.6.0")
    doc.async = False: doc.validateOnParse = False: doc.resolveExternals = False
    doc.setProperty "ProhibitDTD", True
    If Not doc.Load(path) Then Err.Raise 5, "LabIO", "Invalid plan XML: " & doc.parseError.reason
    If doc.documentElement.nodeName <> "excel-lab" Then Err.Raise 5, "LabIO", "Expected excel-lab XML root"
    Set LoadPlan = doc
End Function
Public Function SheetNamed(ByVal wb As Workbook, ByVal name As String) As Worksheet
    Dim ws As Worksheet
    For Each ws In wb.Worksheets
        If StrComp(ws.Name, name, vbBinaryCompare) = 0 Then Set SheetNamed = ws: Exit Function
    Next
End Function
Public Function SecondsSince(ByVal started As Single) As Double
    SecondsSince = Timer - started
    If SecondsSince < 0 Then SecondsSince = SecondsSince + 86400#
End Function

Public Sub ResetTimings()
    Set mTimings = New Collection
End Sub
Public Sub RecordTiming(ByVal phase As String, ByVal sheet As String, ByVal variantName As String, ByVal started As Single)
    If mTimings Is Nothing Then Set mTimings = New Collection
    mTimings.Add "{""phase"":" & J(phase) & ",""sheet"":" & J(sheet) & ",""variant"":" & J(variantName) & _
        ",""seconds"":" & N(SecondsSince(started)) & "}"
End Sub
Public Sub SaveTimings(ByVal path As String)
    If mTimings Is Nothing Then Set mTimings = New Collection
    SaveText path, JoinJson(mTimings)
End Sub
