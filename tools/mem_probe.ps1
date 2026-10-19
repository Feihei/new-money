<#
    编译期内存探针：跑一条 cargo 命令，同时每 300 ms 采一次「空闲物理内存」与
    rustc/link 的私有内存之和，结束时报峰值。用来把「编译把机器吃死」这类问题
    变成两个数，而不是靠感觉调并行度（成因与实测数字见 ARCHITECTURE.md §6.1 坑 32）。

    用法（在仓库根）：
        powershell -NoProfile -ExecutionPolicy Bypass -File tools/mem_probe.ps1 <标签> <cargo 参数...>
        powershell ... -File tools/mem_probe.ps1 capped test -p game_core --no-run

    输出里 `peak_physical_used` 含 Windows 文件缓存，横向比较看 `peak_commit_by_tools`
    —— 那个才是 rustc/link 自己要的内存，也是死机时真正撑爆的数。
#>
param([string]$Label = "run", [Parameter(ValueFromRemainingArguments = $true)][string[]]$CargoArgs)

$ErrorActionPreference = "SilentlyContinue"
$exe = (Get-Command cargo).Source
$dir = Join-Path (Split-Path -Parent $PSScriptRoot) ".tmp"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$log = Join-Path $dir "probe_$Label.log"
$err = Join-Path $dir "probe_$Label.err"

$totalKB = (Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize
$minFreeKB = $totalKB
$maxPrivMB = 0
$peakName = ""

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $exe -ArgumentList $CargoArgs -NoNewWindow -PassThru `
    -RedirectStandardOutput $log -RedirectStandardError $err

while (-not $p.HasExited) {
    $freeKB = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
    if ($freeKB -lt $minFreeKB) { $minFreeKB = $freeKB }
    $ps = @(Get-Process -Name rustc, link, lld-link, cargo -ErrorAction SilentlyContinue)
    $priv = (($ps | Measure-Object -Property PrivateMemorySize64 -Sum).Sum) / 1MB
    if ($priv -gt $maxPrivMB) {
        $maxPrivMB = $priv
        $peakName = ($ps | Sort-Object PrivateMemorySize64 -Descending | Select-Object -First 1).ProcessName
    }
    Start-Sleep -Milliseconds 300
}
$sw.Stop()

$usedGB = ($totalKB - $minFreeKB) / 1MB
Write-Output ("[{0}] exit={1} elapsed={2:N0}s  peak_physical_used={3:N1} GB / {4:N1} GB  peak_commit_by_tools={5:N1} GB (top proc: {6})  procs_seen={7}" -f `
    $Label, $p.ExitCode, $sw.Elapsed.TotalSeconds, $usedGB, ($totalKB / 1MB), ($maxPrivMB / 1024), $peakName, $ps.Count)
