# Capture a QuickTrim window to a PNG.
#
#   powershell -ExecutionPolicy Bypass -File scripts/screenshot.ps1 -Out shot.png [-ProcessId 1234]
#
# Uses PrintWindow with PW_RENDERFULLCONTENT so the capture works even when the
# window is behind other windows. QuickTrim's software renderer paints into the
# window DC, so this is reliable without needing foreground focus.

param(
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$ProcessId = 0
)

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class QtWindow {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr param);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);

    public struct RECT { public int Left, Top, Right, Bottom; }

    public static void RestoreAll(uint pid) {
        EnumWindows((h, p) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            if (owner == pid && IsIconic(h)) ShowWindow(h, 9);   // SW_RESTORE
            return true;
        }, IntPtr.Zero);
    }

    public static IntPtr Largest(uint pid, out string title, out int width, out int height) {
        IntPtr best = IntPtr.Zero;
        int bestArea = 0;
        string bestTitle = "";
        List<IntPtr> all = new List<IntPtr>();
        EnumWindows((h, p) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            if (owner == pid) all.Add(h);
            return true;
        }, IntPtr.Zero);
        foreach (IntPtr h in all) {
            RECT r; GetClientRect(h, out r);
            int area = (r.Right - r.Left) * (r.Bottom - r.Top);
            if (area > bestArea) {
                bestArea = area;
                best = h;
                StringBuilder sb = new StringBuilder(512);
                GetWindowText(h, sb, 512);
                bestTitle = sb.ToString();
            }
        }
        RECT br; GetClientRect(best, out br);
        title = bestTitle;
        width = br.Right - br.Left;
        height = br.Bottom - br.Top;
        return best;
    }
}
"@

if ($ProcessId -eq 0) {
    $proc = Get-Process quicktrim -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if (-not $proc) { throw "QuickTrim is not running." }
    $ProcessId = $proc.Id
}

$title = ""
$w = 0
$h = 0

# A minimised window reports a 0x0 client rect; restore it before measuring.
[QtWindow]::RestoreAll([uint32]$ProcessId)
Start-Sleep -Milliseconds 400

$hwnd = [QtWindow]::Largest([uint32]$ProcessId, [ref]$title, [ref]$w, [ref]$h)
if ($hwnd -eq [IntPtr]::Zero -or $w -le 0 -or $h -le 0) { throw "No window found for process $ProcessId." }

if (-not [QtWindow]::IsWindowVisible($hwnd)) { [QtWindow]::ShowWindow($hwnd, 5) | Out-Null; Start-Sleep -Milliseconds 300 }

# The window must be mapped for the renderer to have painted at least once.
[QtWindow]::ShowWindow($hwnd, 5) | Out-Null
Start-Sleep -Milliseconds 400

$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [QtWindow]::PrintWindow($hwnd, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

"hwnd=$hwnd title='$title' size=${w}x${h} captured=$ok -> $Out"
