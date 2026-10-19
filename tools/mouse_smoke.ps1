<#
真鼠标冒烟：启动 game_app、把**游戏窗口**抢到前台、按墙上时钟依次点几个客户区坐标，然后等它自己退出。

为什么要这么麻烦：合成点击送的是**屏幕坐标**，落在哪一层窗口由 Z 序决定 —— 游戏窗口被 IDE
压住时点击全进了 IDE，而 NM_SHOT 照常截出游戏画面（GPU 侧截屏不看遮挡），于是「图是对的、
数字没变」看着像 UI bug（.agents/ARCHITECTURE.md §6.1 坑 27）。所以第一步必须是
ShowWindow(SW_RESTORE) + BringWindowToTop + SetForegroundWindow，再用
GetClientRect + ClientToScreen 换算客户区原点（窗口每次启动被系统级联，坐标不能写死）。

坑 27 的第三半（P3b 踩到）：**`$proc.MainWindowHandle` 拿到的不是游戏窗口**。cargo 出的
exe 是控制台子系统，`Start-Process` 不给 `-NoNewWindow` 时系统为它另开一个控制台窗口，而
MainWindowHandle 的规则是「第一个有标题的窗口」—— 那个控制台窗口有标题（就是 exe 路径），
winit 的游戏窗口反而排在后面。于是量到的是控制台的 960x480 与它的原点，点击整体偏到一个
从没存在过的坐标上，表现为「按了没反应」。修法：枚举该进程的窗口，挑可见、非控制台、
客户区最大的那个（见 `Get-GameWindow`）。判据：**先确认点的是哪扇窗，再谈点没点到**。

坐标从哪儿来：`NM_SHOT` 那张 PNG 就是客户区的物理像素，用连通块量出按钮包围盒即可，
不需要猜 DPI（脚本会打印 LOGPIXELSX 与缩放；不是 96 时先把点击坐标按缩放换算过再用）。

时间轴：点击时刻按**进程启动**起算，与 `NM_SHOT_AT`（游戏启动起算）同向 —— 游戏比进程晚
零点几秒就绪，所以点击相对游戏时间只会更早，不会落到截图之后。这是坑 27 第二半的解法：
让脚本自己拉起游戏，两个钟就没有错位的可能。

用法（`-File` 调用时 PowerShell 不按逗号拆数组，所以多个值用分号写在一个参数里）：
  powershell -File tools/mouse_smoke.ps1 `
    -Env 'NM_SCENE=board;NM_HAND=1' `
    -Click '2.5:960,38;4.0:1000,425' `
    -Shot .tmp/shots/smoke_play.png -ShotAt 6.0
#>
param(
    [string]$Exe = "target/debug/game_app.exe",
    [string]$Env = "",
    [string]$Click = "",
    [string]$Shot = "",
    [double]$ShotAt = 4.5,
    [string]$Log = ".tmp/smoke_game.log",
    [int]$WindowTimeoutSec = 30
)
$ErrorActionPreference = "Stop"

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x, y; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("gdi32.dll")]  public static extern int GetDeviceCaps(IntPtr hdc, int idx);
}
"@

# 枚举这个进程名下的可见窗口，挑客户区最大且不是控制台的那个 —— MainWindowHandle 会挑错。
# 闭包变量走 $script: 而不是函数局部：委托由 user32 直接回调，拿不到函数作用域。
# 实测这个游戏进程名下的窗口：Window Class（1280x720，要的就是它）、ConsoleWindowClass、
# Winit Thread Event Target（0x0）、temp_d3d_window（1x1）、__wglDummyWindow（不可见）
function Get-GameWindow([uint32]$target) {
    $script:gwTarget = $target
    $script:best = $null
    $cb = {
        param($h, $l)
        $pid2 = [uint32]0
        [void][Win]::GetWindowThreadProcessId($h, [ref]$pid2)
        if ($pid2 -ne $script:gwTarget) { return $true }
        if (-not [Win]::IsWindowVisible($h)) { return $true }
        $cls = New-Object System.Text.StringBuilder 256
        [void][Win]::GetClassName($h, $cls, 256)
        if ($cls.ToString() -eq "ConsoleWindowClass") { return $true }
        $r = New-Object Win+RECT
        [void][Win]::GetClientRect($h, [ref]$r)
        $area = $r.right * $r.bottom
        if ($area -lt 327680) { return $true } # 小于 640x512 的是 winit 的辅助窗口，不是画面对象
        if ($null -eq $script:best -or $area -gt $script:best.Area) {
            $script:best = [pscustomobject]@{ H = $h; W = $r.right; Ht = $r.bottom; Area = $area; Cls = $cls.ToString() }
        }
        return $true
    }
    [void][Win]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:best
}

# 脚本自身的 DPI 感知要先钉住，否则 SetCursorPos 的坐标会被系统虚拟化到别处
[void][Win]::SetProcessDPIAware()
$dc = [Win]::GetDC([IntPtr]::Zero)
$dpi = [Win]::GetDeviceCaps($dc, 88) # LOGPIXELSX
Write-Output ("dpi: {0} (scale {1}%)" -f $dpi, [math]::Round($dpi * 100 / 96))

$root = Split-Path -Parent $PSScriptRoot
$exePath = if ([IO.Path]::IsPathRooted($Exe)) { $Exe } else { Join-Path $root ($Exe -replace "/", "\") }
if (-not (Test-Path $exePath)) { throw "no executable at $exePath (run cargo build -p game_app first)" }
$shotPath = if ($Shot -eq "") { "" } elseif ([IO.Path]::IsPathRooted($Shot)) { $Shot } else { Join-Path $root ($Shot -replace "/", "\") }
$logPath = if ([IO.Path]::IsPathRooted($Log)) { $Log } else { Join-Path $root ($Log -replace "/", "\") }

foreach ($pair in ($Env -split ";" | Where-Object { $_ -ne "" })) {
    $k, $v = $pair -split "=", 2
    Set-Item -Path "Env:$k" -Value $v
    Write-Output ("env {0}={1}" -f $k, $v)
}
if ($Shot -ne "") {
    Set-Item -Path "Env:NM_SHOT" -Value $shotPath
    Set-Item -Path "Env:NM_SHOT_AT" -Value ("{0}" -f $ShotAt)
}

# WorkingDirectory 必须是仓库根：资产根由 CWD 下的 assets/ 决定（见 main.rs 的 asset_root）。
# -NoNewWindow 让游戏沿用当前控制台，日志重定向进文件；否则那个新控制台窗口还会抢走前台焦点
$p = Start-Process -FilePath $exePath -PassThru -NoNewWindow -WorkingDirectory $root `
    -RedirectStandardOutput $logPath -RedirectStandardError "${logPath}.err"
$t0 = Get-Date

$game = $null
while (((Get-Date) - $t0).TotalSeconds -lt $WindowTimeoutSec) {
    $game = Get-GameWindow ([uint32]$p.Id)
    if ($null -ne $game) { break }
    Start-Sleep -Milliseconds 50
}
if ($null -eq $game) { throw "${WindowTimeoutSec}s 内没等到游戏窗口，先查 $logPath" }
$h = $game.H

[void][Win]::ShowWindow($h, 9)          # SW_RESTORE：最小化时坐标全废
[void][Win]::BringWindowToTop($h)
[void][Win]::SetForegroundWindow($h)

$rect = New-Object Win+RECT
$origin = New-Object Win+POINT
[void][Win]::GetClientRect($h, [ref]$rect)
[void][Win]::ClientToScreen($h, [ref]$origin)
if ($rect.right -ne $game.W -or $rect.bottom -ne $game.Ht) {
    # 与枚举时量到的尺寸不一致 = 这中间窗口被动过（DPI 变化、被别的脚本 resize），坐标不可信
    Write-Output ("warn: client changed during setup, {0}x{1} -> {2}x{3}" -f $game.W, $game.Ht, $rect.right, $rect.bottom)
}
Write-Output ("window: class={0} client {1}x{2}, screen origin ({3},{4})" -f $game.Cls, $rect.right, $rect.bottom, $origin.x, $origin.y)

foreach ($spec in ($Click -split ";" | Where-Object { $_ -ne "" })) {
    $at, $xy = $spec -split ":", 2
    $cx, $cy = $xy -split ",", 2
    # 等到「进程启动后第 at 秒」再动鼠标：等待期间不用大颗粒 sleep，抖动会让点击落到截图之后
    while (((Get-Date) - $t0).TotalSeconds -lt [double]$at) { Start-Sleep -Milliseconds 20 }
    $sx = $origin.x + [int]$cx
    $sy = $origin.y + [int]$cy
    [void][Win]::SetCursorPos($sx, $sy)
    Start-Sleep -Milliseconds 40
    [Win]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero) | Out-Null # LEFTDOWN
    [Win]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero) | Out-Null # LEFTUP
    Write-Output ("click {0,5}s -> client ({1},{2}) screen ({3},{4})" -f $at, $cx, $cy, $sx, $sy)
}

# 游戏自己退出（NM_SHOT 落盘后再等 1s），不留下没人关的窗口进程。
# Windows PowerShell 5.1 的 Process 只有 `WaitForExit(int ms)`，没有 TimeSpan 重载
$waitMs = [int]((60 + $ShotAt) * 1000)
if (-not $p.WaitForExit($waitMs)) {
    Stop-Process -Id $p.Id -Force
    Write-Output "warn: 超时未退出，已强制结束（设了 NM_SHOT 才会自行退出）"
}
Write-Output ("exit: {0}, elapsed {1:N1}s" -f $p.ExitCode, ((Get-Date) - $t0).TotalSeconds)
if ($shotPath -ne "") {
    if (Test-Path $shotPath) {
        Write-Output ("shot: {0} ({1:N0} bytes)" -f $shotPath, (Get-Item $shotPath).Length)
    } else {
        Write-Output "shot: NOT WRITTEN - check the shot path and NM_SHOT_AT"
    }
}
