Attribute VB_Name = "LabBounds"
Option Explicit

' Geometry envelope, NOT a proof of all painted pixels (text overflow, shadows,
' glow, clipped OLE contents etc. require the separate rendering experiments).
Public Function Collect(ByVal ws As Worksheet, ByRef envelope As String) As String
    Dim used As Range, sh As Shape, entries As New Collection
    Dim x1 As Double, y1 As Double, x2 As Double, y2 As Double
    Dim l As Double, t As Double, w As Double, h As Double, angle As Double
    Dim cx As Double, cy As Double, rw As Double, rh As Double, j As String
    Set used = ws.UsedRange
    x1 = used.Left: y1 = used.Top: x2 = x1 + used.Width: y2 = y1 + used.Height
    For Each sh In ws.Shapes
        ' Any inaccessible shape fails the measurement, rather than silently
        ' declaring a smaller envelope complete.
        l = sh.Left: t = sh.Top: w = sh.Width: h = sh.Height
        angle = sh.Rotation * 3.14159265358979 / 180#
        rw = Abs(w * Cos(angle)) + Abs(h * Sin(angle))
        rh = Abs(w * Sin(angle)) + Abs(h * Cos(angle))
        cx = l + w / 2#: cy = t + h / 2#
        j = "{""name"":" & LabIO.J(sh.Name) & ",""type"":" & CStr(sh.Type) & _
            ",""visible"":" & LabIO.B(sh.Visible <> 0) & ",""left_points"":" & LabIO.N(l) & _
            ",""top_points"":" & LabIO.N(t) & ",""width_points"":" & LabIO.N(w) & _
            ",""height_points"":" & LabIO.N(h) & ",""rotation_degrees"":" & LabIO.N(sh.Rotation) & _
            ",""top_left_cell"":" & LabIO.J(sh.TopLeftCell.Address) & _
            ",""bottom_right_cell"":" & LabIO.J(sh.BottomRightCell.Address) & "}"
        entries.Add j
        If sh.Visible <> 0 Then
            ' Union unrotated + rotated bounding rectangles conservatively.
            x1 = Application.Min(x1, l, cx - rw / 2#): x2 = Application.Max(x2, l + w, cx + rw / 2#)
            y1 = Application.Min(y1, t, cy - rh / 2#): y2 = Application.Max(y2, t + h, cy + rh / 2#)
        End If
    Next
    envelope = ws.Range(ws.Cells(RowAt(ws, Application.Max(0, y1)), ColAt(ws, Application.Max(0, x1))), _
                        ws.Cells(RowAt(ws, Application.Max(0, y2 - 0.00001)), ColAt(ws, Application.Max(0, x2 - 0.00001)))).Address
    Collect = "{""kind"":""used-range-plus-visible-shape-geometry"",""painted_pixel_bounds_proven"":false," & _
        """left_points"":" & LabIO.N(x1) & ",""top_points"":" & LabIO.N(y1) & _
        ",""right_points"":" & LabIO.N(x2) & ",""bottom_points"":" & LabIO.N(y2) & _
        ",""cell_envelope"":" & LabIO.J(envelope) & ",""shapes"":" & LabIO.JoinJson(entries) & "}"
End Function
Private Function RowAt(ByVal ws As Worksheet, ByVal y As Double) As Long
    Dim lo As Long, hi As Long, mid As Long
    lo = 1: hi = ws.Rows.Count
    Do While lo < hi
        mid = lo + (hi - lo + 1) \ 2
        If ws.Cells(mid, 1).Top <= y Then lo = mid Else hi = mid - 1
    Loop
    RowAt = lo
End Function
Private Function ColAt(ByVal ws As Worksheet, ByVal x As Double) As Long
    Dim lo As Long, hi As Long, mid As Long
    lo = 1: hi = ws.Columns.Count
    Do While lo < hi
        mid = lo + (hi - lo + 1) \ 2
        If ws.Cells(1, mid).Left <= x Then lo = mid Else hi = mid - 1
    Loop
    ColAt = lo
End Function
