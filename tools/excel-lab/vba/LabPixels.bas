Attribute VB_Name = "LabPixels"
Option Explicit

' GDI+ is called in the Excel process: no PowerShell callback per capture/pixel.
' Clipboard handles are borrowed only while OpenClipboard is held. An owned
' pixel clone / owned EMF is created before CloseClipboard.
Private Declare PtrSafe Function GdiplusStartup Lib "gdiplus" (ByRef token As LongPtr, ByRef startupInput As Any, ByVal savedImage As LongPtr) As Long
Private Declare PtrSafe Sub GdiplusShutdown Lib "gdiplus" (ByVal token As LongPtr)
Private Declare PtrSafe Function GdipCreateBitmapFromHBITMAP Lib "gdiplus" (ByVal hBitmap As LongPtr, ByVal palette As LongPtr, ByRef image As LongPtr) As Long
Private Declare PtrSafe Function GdipCloneBitmapAreaI Lib "gdiplus" (ByVal x As Long, ByVal y As Long, ByVal width As Long, ByVal height As Long, ByVal format As Long, ByVal source As LongPtr, ByRef image As LongPtr) As Long
Private Declare PtrSafe Function GdipCreateBitmapFromScan0 Lib "gdiplus" (ByVal width As Long, ByVal height As Long, ByVal stride As Long, ByVal format As Long, ByVal scan0 As LongPtr, ByRef image As LongPtr) As Long
Private Declare PtrSafe Function GdipCreateMetafileFromEmf Lib "gdiplus" (ByVal emf As LongPtr, ByVal deleteEmf As Long, ByRef image As LongPtr) As Long
Private Declare PtrSafe Function GdipGetImageWidth Lib "gdiplus" (ByVal image As LongPtr, ByRef width As Long) As Long
Private Declare PtrSafe Function GdipGetImageHeight Lib "gdiplus" (ByVal image As LongPtr, ByRef height As Long) As Long
Private Declare PtrSafe Function GdipGetImageGraphicsContext Lib "gdiplus" (ByVal image As LongPtr, ByRef graphics As LongPtr) As Long
Private Declare PtrSafe Function GdipGraphicsClear Lib "gdiplus" (ByVal graphics As LongPtr, ByVal color As Long) As Long
Private Declare PtrSafe Function GdipDrawImageRectI Lib "gdiplus" (ByVal graphics As LongPtr, ByVal image As LongPtr, ByVal x As Long, ByVal y As Long, ByVal width As Long, ByVal height As Long) As Long
Private Declare PtrSafe Function GdipSetInterpolationMode Lib "gdiplus" (ByVal graphics As LongPtr, ByVal mode As Long) As Long
Private Declare PtrSafe Function GdipSetPixelOffsetMode Lib "gdiplus" (ByVal graphics As LongPtr, ByVal mode As Long) As Long
Private Declare PtrSafe Function GdipDeleteGraphics Lib "gdiplus" (ByVal graphics As LongPtr) As Long
Private Declare PtrSafe Function GdipDisposeImage Lib "gdiplus" (ByVal image As LongPtr) As Long
Private Declare PtrSafe Function GdipBitmapLockBits Lib "gdiplus" (ByVal image As LongPtr, ByRef rect As Any, ByVal flags As Long, ByVal format As Long, ByRef data As Any) As Long
Private Declare PtrSafe Function GdipBitmapUnlockBits Lib "gdiplus" (ByVal image As LongPtr, ByRef data As Any) As Long
Private Declare PtrSafe Function GdipSaveImageToFile Lib "gdiplus" (ByVal image As LongPtr, ByVal fileName As LongPtr, ByRef encoder As Any, ByVal params As LongPtr) As Long
Private Declare PtrSafe Function CLSIDFromString Lib "ole32" (ByVal text As LongPtr, ByRef guid As Any) As Long
Private Declare PtrSafe Function OpenClipboard Lib "user32" (ByVal hwnd As LongPtr) As Long
Private Declare PtrSafe Function CloseClipboard Lib "user32" () As Long
Private Declare PtrSafe Function GetClipboardData Lib "user32" (ByVal format As Long) As LongPtr
Private Declare PtrSafe Function GetClipboardSequenceNumber Lib "user32" () As Long
Private Declare PtrSafe Function GetClipboardOwner Lib "user32" () As LongPtr
Private Declare PtrSafe Function GetWindowThreadProcessId Lib "user32" (ByVal hwnd As LongPtr, ByRef processId As Long) As Long
Private Declare PtrSafe Function GetCurrentProcessId Lib "kernel32" () As Long
Private Declare PtrSafe Sub Sleep Lib "kernel32" (ByVal milliseconds As Long)
Private Declare PtrSafe Function CopyEnhMetaFileW Lib "gdi32" (ByVal source As LongPtr, ByVal fileName As LongPtr) As LongPtr
Private Declare PtrSafe Function DeleteEnhMetaFile Lib "gdi32" (ByVal emf As LongPtr) As Long
Private Declare PtrSafe Function GetEnhMetaFileHeader Lib "gdi32" (ByVal emf As LongPtr, ByVal size As Long, ByRef header As Any) As Long

Private Type Rect
    X As Long
    Y As Long
    Width As Long
    Height As Long
End Type
Private Type BitmapData
    Width As Long
    Height As Long
    Stride As Long
    PixelFormat As Long
    Scan0 As LongPtr
    Reserved As LongPtr
End Type
Private mToken As LongPtr
Private Const ARGB32 As Long = &H26200A
Private Const RGB24 As Long = &H21808
Public LastClipboardFormat As String
Public LastSequence As Long
Public MaxPixels As Double

Private Sub Check(ByVal status As Long, ByVal operation As String)
    If status <> 0 Then Err.Raise vbObjectError + 810, "LabPixels", operation & " GDI+ status=" & CStr(status)
End Sub
Public Sub Initialize()
    ' Explicit byte layout avoids UDT alignment ambiguity between Office x86/x64.
    Dim startupInput(0 To 23) As Byte
    If mToken <> 0 Then Exit Sub
    startupInput(0) = 1
    Check GdiplusStartup(mToken, startupInput(0), 0), "GdiplusStartup"
    If MaxPixels <= 0 Then MaxPixels = 32000000#
End Sub
Public Sub Shutdown()
    If mToken <> 0 Then GdiplusShutdown mToken: mToken = 0
End Sub
Public Sub Dispose(ByRef image As LongPtr)
    If image <> 0 Then GdipDisposeImage image: image = 0
End Sub
Public Sub Size(ByVal image As LongPtr, ByRef w As Long, ByRef h As Long)
    Check GdipGetImageWidth(image, w), "Image width": Check GdipGetImageHeight(image, h), "Image height"
    If w < 1 Or h < 1 Or w >= 32766 Or h >= 32766 Or CDbl(w) * h > MaxPixels Then _
        Err.Raise 5, "LabPixels", "Unsafe/possibly truncated image size: " & CStr(w) & "x" & CStr(h)
End Sub
Public Function Crop(ByVal image As LongPtr, ByVal x As Long, ByVal y As Long, ByVal width As Long, ByVal height As Long) As LongPtr
    Dim w As Long, h As Long
    Size image, w, h
    If x < 0 Or y < 0 Or width < 1 Or height < 1 Or CDbl(x) + width > w Or CDbl(y) + height > h Then _
        Err.Raise 5, "LabPixels", "Pixel crop outside image"
    Check GdipCloneBitmapAreaI(x, y, width, height, ARGB32, image, Crop), "Clone pixel crop"
End Function
Private Function FromEmf(ByRef owned As LongPtr, ByVal transparent As Boolean, ByVal dpi As Long) As LongPtr
    Dim meta As LongPtr, image As LongPtr, graphics As LongPtr, header(0 To 21) As Long
    Dim w As Long, h As Long, background As Long, message As String, code As Long
    On Error GoTo Failed
    If GetEnhMetaFileHeader(owned, 88, header(0)) < 88 Then Err.Raise 5, , "Incomplete EMF header"
    ' rclFrame starts at byte 24, units 0.01 mm. Raster scale is explicit.
    w = CLng((CDbl(header(8)) - header(6)) * dpi / 2540#)
    h = CLng((CDbl(header(9)) - header(7)) * dpi / 2540#)
    If w < 1 Or h < 1 Or w >= 32766 Or h >= 32766 Or CDbl(w) * h > MaxPixels Then Err.Raise 5, , "Unsafe EMF frame size"
    Check GdipCreateMetafileFromEmf(owned, 1, meta), "Own EMF"
    owned = 0 ' GDI+ now owns and frees the duplicated EMF handle.
    Check GdipCreateBitmapFromScan0(w, h, 0, ARGB32, 0, image), "Create EMF raster"
    Check GdipGetImageGraphicsContext(image, graphics), "EMF graphics"
    background = &HFFFFFFFF: If transparent Then background = 0
    Check GdipGraphicsClear(graphics, background), "Clear raster canvas"
    Check GdipDrawImageRectI(graphics, meta, 0, 0, w, h), "Render EMF"
    GdipDeleteGraphics graphics: graphics = 0: Dispose meta
    FromEmf = image: Exit Function
Failed:
    code = Err.Number: message = Err.Description
    If graphics <> 0 Then GdipDeleteGraphics graphics
    Dispose meta: Dispose image
    Err.Raise code, "LabPixels.FromEmf", message
End Function
Public Function Capture(ByVal rg As Range, ByVal appearance As Long, ByVal format As Long, ByVal transparent As Boolean, Optional ByVal emfDpi As Long = 96) As LongPtr
    Dim before As Long, ownerPid As Long, started As Single, opened As Boolean
    Dim borrowed As LongPtr, raw As LongPtr, image As LongPtr, ownedEmf As LongPtr
    Dim w As Long, h As Long, attempt As Long, message As String, code As Long
    Initialize
    If appearance <> xlScreen And appearance <> xlPrinter Then Err.Raise 5, , "Invalid CopyPicture appearance"
    If format <> xlBitmap And format <> xlPicture Then Err.Raise 5, , "Invalid CopyPicture format"
    If transparent And format <> xlPicture Then Err.Raise 5, , "Transparent-canvas experiment requires xlPicture/EMF"
    If rg.Areas.Count <> 1 Or rg.Width <= 0 Or rg.Height <= 0 Then Err.Raise 5, , "Empty/noncontiguous capture range"
    On Error GoTo Failed
    rg.Worksheet.Parent.Activate: rg.Worksheet.Activate
    Application.ActiveWindow.DisplayGridlines = False
    Application.ActiveWindow.Zoom = 100
    rg.Select
    For attempt = 1 To 3
        before = GetClipboardSequenceNumber()
        rg.CopyPicture appearance, format
        started = Timer
        Do
            DoEvents
            If GetClipboardSequenceNumber() <> before Then
                ownerPid = 0: GetWindowThreadProcessId GetClipboardOwner(), ownerPid
                If ownerPid = GetCurrentProcessId() Then
                    If OpenClipboard(CLngPtr(Application.hwnd)) <> 0 Then
                        opened = True
                        ' Recheck under the lock; never consume another application's image.
                        ownerPid = 0: GetWindowThreadProcessId GetClipboardOwner(), ownerPid
                        If ownerPid = GetCurrentProcessId() Then
                            If format = xlPicture Then
                                borrowed = GetClipboardData(14) ' CF_ENHMETAFILE
                                If borrowed <> 0 Then
                                    ownedEmf = CopyEnhMetaFileW(borrowed, 0)
                                    If ownedEmf = 0 Then Err.Raise 5, , "CopyEnhMetaFileW failed"
                                    LastClipboardFormat = "CF_ENHMETAFILE"
                                End If
                            Else
                                borrowed = GetClipboardData(2) ' CF_BITMAP, including synthesized DIB
                                If borrowed <> 0 Then
                                    Check GdipCreateBitmapFromHBITMAP(borrowed, 0, raw), "Read clipboard bitmap"
                                    Size raw, w, h
                                    Check GdipCloneBitmapAreaI(0, 0, w, h, ARGB32, raw, image), "Own pixel clone"
                                    Dispose raw
                                    LastClipboardFormat = "CF_BITMAP (opaque; source alpha not retained)"
                                End If
                            End If
                            LastSequence = GetClipboardSequenceNumber()
                        End If
                        CloseClipboard: opened = False
                        If ownedEmf <> 0 Then image = FromEmf(ownedEmf, transparent, emfDpi)
                        If image <> 0 Then Size image, w, h: Capture = image: Exit Function
                    End If
                End If
            End If
            ' Only wait on an actual delayed/busy clipboard; no unconditional
            ' pause per cell or per successful capture.
            Sleep 10
        Loop While LabIO.SecondsSince(started) < 3#
    Next
    Err.Raise 5, , "CopyPicture returned no fresh, Excel-owned image after retries"
Failed:
    code = Err.Number: message = Err.Description
    Dispose raw
    If opened Then CloseClipboard
    If ownedEmf <> 0 Then DeleteEnhMetaFile ownedEmf
    Dispose image
    Err.Raise code, "LabPixels.Capture", message
End Function
Public Sub Save(ByVal image As LongPtr, ByVal path As String, ByVal format As String)
    Dim guid(0 To 15) As Byte, clsid As String, temp As String, fso As Object
    Dim flattened As LongPtr, graphics As LongPtr, w As Long, h As Long, savedImage As LongPtr
    Dim code As Long, message As String
    On Error GoTo Failed
    Set fso = CreateObject("Scripting.FileSystemObject")
    If fso.FileExists(path) Or fso.FileExists(path & ".partial") Then Err.Raise 58, , "Image exists: " & path
    savedImage = image
    If format = "png" Then
        clsid = "{557CF406-1A04-11D3-9A73-0000F81EF32E}"
    ElseIf format = "bmp" Then
        ' Explicit RGB24/white policy, not ambiguous BMP alpha semantics.
        clsid = "{557CF400-1A04-11D3-9A73-0000F81EF32E}"
        Size image, w, h
        Check GdipCreateBitmapFromScan0(w, h, 0, RGB24, 0, flattened), "BMP RGB24"
        Check GdipGetImageGraphicsContext(flattened, graphics), "BMP graphics"
        Check GdipGraphicsClear(graphics, &HFFFFFFFF), "BMP white background"
        Check GdipSetInterpolationMode(graphics, 5), "BMP interpolation"
        Check GdipSetPixelOffsetMode(graphics, 4), "BMP pixel offset"
        Check GdipDrawImageRectI(graphics, image, 0, 0, w, h), "BMP composite"
        GdipDeleteGraphics graphics: graphics = 0: savedImage = flattened
    Else
        Err.Raise 5, , "Expected png or bmp"
    End If
    Check CLSIDFromString(StrPtr(clsid), guid(0)), "Encoder CLSID"
    temp = path & ".partial"
    Check GdipSaveImageToFile(savedImage, StrPtr(temp), guid(0), 0), "Save image"
    Dispose flattened
    fso.MoveFile temp, path
    Exit Sub
Failed:
    code = Err.Number: message = Err.Description
    If graphics <> 0 Then GdipDeleteGraphics graphics
    Dispose flattened
    Err.Raise code, "LabPixels.Save", message
End Sub
Public Function PixelHash(ByVal image As LongPtr) As String
    Dim rect As Rect, data As BitmapData, state As LabHash.State, locked As Boolean
    Dim y As Long, code As Long, message As String
    On Error GoTo Failed
    Size image, rect.Width, rect.Height
    LabHash.Start state
    LabHash.Feed state, VarPtr(rect.Width), 4: LabHash.Feed state, VarPtr(rect.Height), 4
    Check GdipBitmapLockBits(image, rect, 1, ARGB32, data), "Lock pixels": locked = True
    For y = 0 To rect.Height - 1
        LabHash.Feed state, data.Scan0 + CLngPtr(y) * data.Stride, rect.Width * 4
    Next
    Check GdipBitmapUnlockBits(image, data), "Unlock pixels": locked = False
    PixelHash = LabHash.Finish(state): Exit Function
Failed:
    code = Err.Number: message = Err.Description
    If locked Then GdipBitmapUnlockBits image, data
    LabHash.Dispose state
    Err.Raise code, "LabPixels.PixelHash", message
End Function
Public Function Compare(ByVal a As LongPtr, ByVal b As LongPtr, Optional ByVal tolerance As Long = 0) As String
    Dim ra As Rect, rb As Rect, da As BitmapData, db As BitmapData, la As Boolean, lb As Boolean
    Dim rowA() As Byte, rowB() As Byte, x As Long, y As Long, channel As Long, offset As Long
    Dim changed As Double, different As Boolean, code As Long, message As String
    If tolerance < 0 Or tolerance > 255 Then Err.Raise 5, , "Invalid pixel tolerance"
    On Error GoTo Failed
    Size a, ra.Width, ra.Height: Size b, rb.Width, rb.Height
    If ra.Width <> rb.Width Or ra.Height <> rb.Height Then
        Compare = "{""equal"":false,""reason"":""dimensions"",""original_width"":" & CStr(ra.Width) & ",""original_height"":" & CStr(ra.Height) & _
                  ",""shrunk_width"":" & CStr(rb.Width) & ",""shrunk_height"":" & CStr(rb.Height) & "}": Exit Function
    End If
    If PixelHash(a) = PixelHash(b) Then
        Compare = "{""equal"":true,""changed_pixels"":0,""method"":""sha256-pixel-prefilter""}": Exit Function
    End If
    Check GdipBitmapLockBits(a, ra, 1, ARGB32, da), "Lock original": la = True
    Check GdipBitmapLockBits(b, rb, 1, ARGB32, db), "Lock shrunk": lb = True
    ReDim rowA(0 To ra.Width * 4 - 1): ReDim rowB(0 To ra.Width * 4 - 1)
    For y = 0 To ra.Height - 1
        LabHash.CopyBytes rowA(0), ByVal da.Scan0 + CLngPtr(y) * da.Stride, ra.Width * 4
        LabHash.CopyBytes rowB(0), ByVal db.Scan0 + CLngPtr(y) * db.Stride, ra.Width * 4
        For x = 0 To ra.Width - 1
            offset = x * 4: different = False
            For channel = 0 To 3
                If Abs(CLng(rowA(offset + channel)) - CLng(rowB(offset + channel))) > tolerance Then different = True: Exit For
            Next
            If different Then changed = changed + 1
        Next
    Next
    GdipBitmapUnlockBits a, da: la = False: GdipBitmapUnlockBits b, db: lb = False
    Compare = "{""equal"":" & LabIO.B(changed = 0) & ",""changed_pixels"":" & LabIO.N(changed) & ",""tolerance"":" & CStr(tolerance) & ",""method"":""VBA-buffer-RGBA-compare""}": Exit Function
Failed:
    code = Err.Number: message = Err.Description
    If la Then GdipBitmapUnlockBits a, da
    If lb Then GdipBitmapUnlockBits b, db
    Err.Raise code, "LabPixels.Compare", message
End Function
' Experimental nonuniform-pixel probe, not a general text-overflow proof.
Public Function NonUniform(ByVal image As LongPtr, Optional ByVal border As Long = 1, Optional ByVal tolerance As Long = 14) As Boolean
    Dim rect As Rect, data As BitmapData, locked As Boolean, row() As Byte, bg(0 To 3) As Byte
    Dim x As Long, y As Long, ch As Long, code As Long, message As String
    On Error GoTo Failed
    Size image, rect.Width, rect.Height
    If border < 0 Or rect.Width <= border * 2 Or rect.Height <= border * 2 Then Err.Raise 5, , "Pixel probe too small"
    Check GdipBitmapLockBits(image, rect, 1, ARGB32, data), "Lock blankness probe": locked = True
    LabHash.CopyBytes bg(0), ByVal data.Scan0 + CLngPtr(border) * data.Stride + border * 4, 4
    ReDim row(0 To rect.Width * 4 - 1)
    For y = border To rect.Height - border - 1
        LabHash.CopyBytes row(0), ByVal data.Scan0 + CLngPtr(y) * data.Stride, rect.Width * 4
        For x = border To rect.Width - border - 1
            For ch = 0 To 3
                If Abs(CLng(row(x * 4 + ch)) - CLng(bg(ch))) > tolerance Then NonUniform = True: GoTo Done
            Next
        Next
    Next
Done:
    GdipBitmapUnlockBits image, data: Exit Function
Failed:
    code = Err.Number: message = Err.Description
    If locked Then GdipBitmapUnlockBits image, data
    Err.Raise code, "LabPixels.NonUniform", message
End Function

Public Function Solid(ByVal width As Long, ByVal height As Long, ByVal argb As Long) As LongPtr
    Dim graphics As LongPtr, image As LongPtr, code As Long, message As String
    On Error GoTo Failed
    Initialize
    If width < 1 Or height < 1 Or CDbl(width) * height > MaxPixels Then Err.Raise 5, , "Invalid fixture image"
    Check GdipCreateBitmapFromScan0(width, height, 0, ARGB32, 0, image), "Create solid fixture"
    Check GdipGetImageGraphicsContext(image, graphics), "Fixture graphics"
    Check GdipGraphicsClear(graphics, argb), "Fill fixture"
    GdipDeleteGraphics graphics: Solid = image: Exit Function
Failed:
    code = Err.Number: message = Err.Description
    If graphics <> 0 Then GdipDeleteGraphics graphics
    Dispose image
    Err.Raise code, "LabPixels.Solid", message
End Function
