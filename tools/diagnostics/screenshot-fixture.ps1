# Synthetic content only; run with a task-owned directory outside the repository.
param([Parameter(Mandatory=$true)][string]$OutDirectory)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VwFixtureDpi {
    [DllImport("user32.dll", SetLastError=true)] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
}
'@
if (-not [VwFixtureDpi]::SetProcessDpiAwarenessContext([IntPtr](-4))) { throw 'Fixture PMv2 initialization failed' }
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;
public class VwCaptureFixture : Form {
    private readonly string receipt;
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out Rect rect, int size);
    public VwCaptureFixture(int index, Rectangle screen, string directory) {
        receipt = Path.Combine(directory, "fixture-" + index + ".json");
        Text = "VW Synthetic Capture Monitor " + index;
        FormBorderStyle = FormBorderStyle.None;
        AutoScaleMode = AutoScaleMode.None;
        StartPosition = FormStartPosition.Manual;
        Bounds = new Rectangle(screen.Left + 40, screen.Top + 40, 900, 600);
        BackColor = Color.White;
    }
    protected override void OnPaint(PaintEventArgs e) {
        base.OnPaint(e);
        int w = ClientSize.Width, h = ClientSize.Height;
        using (Brush red = new SolidBrush(Color.FromArgb(240, 20, 20)))
        using (Brush green = new SolidBrush(Color.FromArgb(20, 200, 20)))
        using (Brush blue = new SolidBrush(Color.FromArgb(20, 20, 240)))
        using (Brush yellow = new SolidBrush(Color.FromArgb(240, 220, 20))) {
            e.Graphics.FillRectangle(red, 0, 0, w/2, h/2);
            e.Graphics.FillRectangle(green, w/2, 0, w-w/2, h/2);
            e.Graphics.FillRectangle(blue, 0, h/2, w/2, h-h/2);
            e.Graphics.FillRectangle(yellow, w/2, h/2, w-w/2, h-h/2);
        }
    }
    protected override void OnShown(EventArgs e) {
        base.OnShown(e);
        Rect bounds;
        if (DwmGetWindowAttribute(Handle, 9, out bounds, 16) != 0) throw new Exception("Fixture DWM query failed");
        File.WriteAllText(receipt, "{\"width\":" + (bounds.Right-bounds.Left) +
            ",\"height\":" + (bounds.Bottom-bounds.Top) + ",\"dpi\":" + GetDpiForWindow(Handle) + "}");
    }
}
'@
$forms = @()
$index = 0
foreach ($screen in [Windows.Forms.Screen]::AllScreens | Select-Object -First 2) {
    $index++
    $form = New-Object VwCaptureFixture($index, $screen.Bounds, $OutDirectory)
    $forms += $form
    $form.Show()
}
$context = New-Object Windows.Forms.ApplicationContext
$timer = New-Object Windows.Forms.Timer
$timer.Interval = 500
$timer.Add_Tick({
    if (Test-Path -LiteralPath (Join-Path $OutDirectory 'stop')) {
        foreach ($form in $forms) { $form.Close(); $form.Dispose() }
        $timer.Stop()
        $context.ExitThread()
    }
})
$timer.Start()
try { [Windows.Forms.Application]::Run($context) }
finally { $timer.Dispose(); $context.Dispose(); foreach ($form in $forms) { $form.Dispose() } }
