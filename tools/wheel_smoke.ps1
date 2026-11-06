# One-off verification helper: send mouse wheel to the game window (screen route, needs foreground).
# Usage: powershell -ExecutionPolicy Bypass -File tools/wheel_smoke.ps1 -Delay 6 -Notches -3
param(
    [int]$Delay = 6,
    [int]$Notches = -3,
    [string]$Route = "screen"
)

Add-Type -Namespace Native -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
[DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, out POINT p);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
public struct RECT { public int L, T, R, B; }
public struct POINT { public int X, Y; }
'@

Start-Sleep -Seconds $Delay
$deadline = (Get-Date).AddSeconds(120)
$proc = $null
while ($null -eq $proc -and (Get-Date) -lt $deadline) {
    $proc = Get-Process | Where-Object { $_.MainWindowTitle -like "*B5*" } | Select-Object -First 1
    if ($null -eq $proc) { Start-Sleep -Seconds 2 }
}
if ($null -eq $proc) {
    Write-Output "window not found"
    exit 1
}
$hwnd = $proc.MainWindowHandle
[Native.Win]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object Native.Win+RECT
[Native.Win]::GetClientRect($hwnd, [ref]$r) | Out-Null
$p = New-Object Native.Win+POINT
[Native.Win]::ClientToScreen($hwnd, [ref]$p) | Out-Null
$cx = $p.X + [int](($r.R - $r.L) / 2)
$cy = $p.Y + [int](($r.B - $r.T) / 2)
[Native.Win]::SetCursorPos($cx, $cy) | Out-Null
Start-Sleep -Milliseconds 300
for ($i = 0; $i -lt [Math]::Abs($Notches); $i++) {
    $delta = if ($Notches -lt 0) { -120 } else { 120 }
    if ($Route -eq "post") {
        $w = [IntPtr]($delta * 65536)
        $l = [IntPtr](($cy - $p.Y) * 65536 + ($cx - $p.X))
        [Native.Win]::PostMessageW($hwnd, 0x020A, $w, $l) | Out-Null
    } else {
        $flag = if ($Notches -lt 0) { 0x0800 } else { 0x0780 }
        [Native.Win]::mouse_event($flag, 0, 0, 0, [IntPtr]::Zero) | Out-Null
    }
    Start-Sleep -Milliseconds 200
}
Write-Output "sent $Notches notches ($Route) at $cx,$cy"
