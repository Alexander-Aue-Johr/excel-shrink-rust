Attribute VB_Name = "LabHash"
Option Explicit

' Windows CNG does SHA-256 in native code. VBA only feeds bounded canonical buffers.
Private Declare PtrSafe Function BCryptOpenAlgorithmProvider Lib "bcrypt.dll" (ByRef alg As LongPtr, ByVal algName As LongPtr, ByVal provider As LongPtr, ByVal flags As Long) As Long
Private Declare PtrSafe Function BCryptCreateHash Lib "bcrypt.dll" (ByVal alg As LongPtr, ByRef hash As LongPtr, ByVal obj As LongPtr, ByVal objBytes As Long, ByVal secret As LongPtr, ByVal secretBytes As Long, ByVal flags As Long) As Long
Private Declare PtrSafe Function BCryptHashData Lib "bcrypt.dll" (ByVal hash As LongPtr, ByVal bytes As LongPtr, ByVal count As Long, ByVal flags As Long) As Long
Private Declare PtrSafe Function BCryptFinishHash Lib "bcrypt.dll" (ByVal hash As LongPtr, ByRef bytes As Any, ByVal count As Long, ByVal flags As Long) As Long
Private Declare PtrSafe Function BCryptDestroyHash Lib "bcrypt.dll" (ByVal hash As LongPtr) As Long
Private Declare PtrSafe Function BCryptCloseAlgorithmProvider Lib "bcrypt.dll" (ByVal alg As LongPtr, ByVal flags As Long) As Long
Public Declare PtrSafe Sub CopyBytes Lib "kernel32" Alias "RtlMoveMemory" (ByRef destination As Any, ByRef source As Any, ByVal count As LongPtr)

Public Type State
    Algorithm As LongPtr
    Handle As LongPtr
End Type
Private Sub Check(ByVal status As Long, ByVal operation As String)
    If status <> 0 Then Err.Raise vbObjectError + 801, "LabHash", operation & " NTSTATUS=0x" & Hex$(status)
End Sub
Public Sub Start(ByRef state As State)
    Dim algorithmName As String: algorithmName = "SHA256"
    Check BCryptOpenAlgorithmProvider(state.Algorithm, StrPtr(algorithmName), 0, 0), "Open SHA256"
    ' Windows 7+: CNG owns the object buffer if NULL/0 are supplied.
    Check BCryptCreateHash(state.Algorithm, state.Handle, 0, 0, 0, 0, 0), "Create SHA256"
End Sub
Public Sub Feed(ByRef state As State, ByVal pointer As LongPtr, ByVal count As Long)
    If count > 0 Then Check BCryptHashData(state.Handle, pointer, count, 0), "HashData"
End Sub
Public Function Finish(ByRef state As State) As String
    Dim digest(0 To 31) As Byte, i As Long, s As String
    Check BCryptFinishHash(state.Handle, digest(0), 32, 0), "Finish SHA256"
    For i = 0 To 31: s = s & LCase$(Right$("0" & Hex$(digest(i)), 2)): Next
    Dispose state: Finish = s
End Function
Public Sub Dispose(ByRef state As State)
    If state.Handle <> 0 Then BCryptDestroyHash state.Handle: state.Handle = 0
    If state.Algorithm <> 0 Then BCryptCloseAlgorithmProvider state.Algorithm, 0: state.Algorithm = 0
End Sub
Public Function ErrorCode(ByRef value As Variant) As Long
    Dim scode As Long
    If VarType(value) <> vbError Then Err.Raise 5, "LabHash", "Not a VT_ERROR"
    ' Windows VARIANT: VT_ERROR's 32-bit SCODE is at offset 8 on Win32 and Win64.
    CopyBytes scode, ByVal VarPtr(value) + 8, 4
    ' Excel stores CVErr(n) as HRESULT &H800A0000 Or n.
    If (scode And &HFFFF0000) = &H800A0000 Then
        ErrorCode = scode And &HFFFF&
    Else
        ErrorCode = scode
    End If
End Function
Public Function NumberBits(ByVal value As Double) As String
    Dim bytes(0 To 7) As Byte, i As Long
    If value = 0 Then value = 0# ' canonical positive zero
    CopyBytes bytes(0), value, 8
    For i = 0 To 7: NumberBits = NumberBits & LCase$(Right$("0" & Hex$(bytes(i)), 2)): Next
End Function
Public Sub SelfTest()
    Dim state As State, bytes(0 To 2) As Byte, v As Variant, message As String, code As Long
    On Error GoTo Failed
    Start state
    If Finish(state) <> "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" Then Err.Raise 5, , "SHA256 empty-vector failure"
    bytes(0) = 97: bytes(1) = 98: bytes(2) = 99
    Start state: Feed state, VarPtr(bytes(0)), 1: Feed state, VarPtr(bytes(1)), 2
    If Finish(state) <> "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" Then Err.Raise 5, , "SHA256 abc-vector failure"
    v = CVErr(xlErrNA)
    If ErrorCode(v) <> xlErrNA Then Err.Raise 5, , "VARIANT SCODE layout failure"
    If NumberBits(1#) <> "000000000000f03f" Then Err.Raise 5, , "IEEE754 layout failure"
    Exit Sub
Failed:
    code = Err.Number: message = Err.Description: Dispose state
    Err.Raise code, "LabHash.SelfTest", message
End Sub
