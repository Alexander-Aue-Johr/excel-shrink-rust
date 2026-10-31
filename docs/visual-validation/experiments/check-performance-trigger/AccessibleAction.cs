using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using Accessibility;

/// <summary>Invoke a uniquely named MSAA action within one owned native pane.</summary>
public static class AccessibleAction
{
    private const int StaticTextRole = 41;
    public static readonly List<string> Candidates = new List<string>();
    [DllImport("oleacc.dll")]
    private static extern int AccessibleObjectFromWindow(
        IntPtr window, uint objectId, ref Guid interfaceId,
        [MarshalAs(UnmanagedType.Interface)] out IAccessible accessible);

    [DllImport("oleacc.dll")]
    private static extern int AccessibleChildren(
        IAccessible parent, int start, int count,
        [Out, MarshalAs(UnmanagedType.LPArray, SizeParamIndex = 2)] object[] children,
        out int obtained);

    public static int Invoke(IntPtr window, string name)
    {
        Guid interfaceId = typeof(IAccessible).GUID;
        IAccessible root;
        Marshal.ThrowExceptionForHR(AccessibleObjectFromWindow(
            window, unchecked((uint)-4), ref interfaceId, out root));
        var matches = new List<Action>();
        Find(root, name, matches, 0);
        if (matches.Count == 1) matches[0]();
        return matches.Count;
    }

    private static void Find(IAccessible parent, string name, List<Action> matches, int depth)
    {
        if (depth > 30) throw new InvalidOperationException("Accessibility tree is too deep.");
        if (parent.get_accName(0) == name)
        {
            string action = parent.get_accDefaultAction(0);
            Candidates.Add("role=" + parent.get_accRole(0) + "; action=" + action);
            if (Convert.ToInt32(parent.get_accRole(0)) != StaticTextRole)
                matches.Add(() => parent.accDoDefaultAction(0));
        }
        int count = parent.accChildCount;
        if (count > 2000) throw new InvalidOperationException("Accessibility subtree is too large.");
        var children = new object[count];
        int obtained;
        Marshal.ThrowExceptionForHR(AccessibleChildren(parent, 0, count, children, out obtained));
        for (int index = 0; index < obtained; index++)
        {
            var accessible = children[index] as IAccessible;
            if (accessible != null)
            {
                Find(accessible, name, matches, depth + 1);
            }
            else if (children[index] is int)
            {
                int childId = (int)children[index];
                if (parent.get_accName(childId) == name)
                {
                    string action = parent.get_accDefaultAction(childId);
                    Candidates.Add("role=" + parent.get_accRole(childId) + "; action=" + action);
                    if (Convert.ToInt32(parent.get_accRole(childId)) != StaticTextRole)
                        matches.Add(() => parent.accDoDefaultAction(childId));
                }
            }
        }
    }
}
