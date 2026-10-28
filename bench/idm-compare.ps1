<#
.SYNOPSIS
    Compares BoltGet with Internet Download Manager (IDM) on the targets in
    bench/targets.toml: same machine, same files, interleaved runs.

.DESCRIPTION
    For every target and round, each tool downloads the file once; the order of
    the tools rotates from round to round (A B C, B C A, C A B, ...). Times are
    wall-clock, measured the same way for both tools from outside:

      IDM      from handing the link to IDM (its COM interface, the one browser
               integrations use) until the final file exists with the expected
               size and IDM has closed it (an exclusive open succeeds).
      BoltGet  from starting `boltget get --no-state` until the process exits
               (the file is complete under its final name by then).

    Every file's SHA-256 is checked after the clock stops, then the file is
    deleted. Lab targets run against a local boltget-testserver, whose /_stats
    counters show how many connections each tool really opened.

    IDM's settings are not changed. They are read from
    HKCU\Software\DownloadManager and recorded in environment.json.

    Requires Windows PowerShell 5.1+, IDM installed and running, and release
    builds: cargo build --release -p boltget-cli -p boltget-testserver

.PARAMETER Targets
    Target names from bench/targets.toml.
.PARAMETER Runs
    Measured rounds per target.
.PARAMETER Warmup
    Warm-up rounds per target (recorded, not counted).
.PARAMETER BoltGetSettings
    BoltGet connection settings: 'auto' (the default of boltget get) or a count.
.PARAMETER Dir
    Scratch directory for the downloads (emptied of payloads afterwards).
.PARAMETER Out
    Directory for the raw data: runs.csv, summary.csv, environment.json.
.PARAMETER TimeoutSec
    Give up on one download after this long.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File bench\idm-compare.ps1 -Dir $env:TEMP\bench -Out bench\results\2026-10-28
#>
[CmdletBinding()]
param(
    [string[]]$Targets = @('cachefly-10mb', 'cachefly-100mb', 'nforce-100mb', 'lab-throttled-100mib', 'lab-1gib'),
    [int]$Runs = 5,
    [int]$Warmup = 1,
    [string[]]$BoltGetSettings = @('auto', '8'),
    [Parameter(Mandatory = $true)][string]$Dir,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$TimeoutSec = 600,
    [int]$LabPort = 8090,
    [string]$IdmPath = 'C:\Program Files (x86)\Internet Download Manager\IDMan.exe'
)

$ErrorActionPreference = 'Stop'
# Raw data uses '.' decimals whatever the system locale.
[System.Threading.Thread]::CurrentThread.CurrentCulture = [Globalization.CultureInfo]::InvariantCulture
[System.Threading.Thread]::CurrentThread.CurrentUICulture = [Globalization.CultureInfo]::InvariantCulture
Set-StrictMode -Version 3
# `powershell -File` passes 'a,b' as one string.
$Targets = @($Targets | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$BoltGetSettings = @($BoltGetSettings | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

$repo = Split-Path -Parent $PSScriptRoot
$boltget = Join-Path $repo 'target\release\boltget.exe'
$testserver = Join-Path $repo 'target\release\boltget-testserver.exe'
$targetsFile = Join-Path $PSScriptRoot 'targets.toml'
foreach ($p in @($boltget, $testserver, $IdmPath, $targetsFile)) {
    if (-not (Test-Path $p)) { throw "missing $p (build with: cargo build --release -p boltget-cli -p boltget-testserver)" }
}
if (-not (Get-Process IDMan -ErrorAction SilentlyContinue)) { throw 'IDM is not running; start it first' }
New-Item -ItemType Directory -Force -Path $Dir, $Out | Out-Null
$Dir = (Resolve-Path $Dir).Path
$Out = (Resolve-Path $Out).Path
# An empty state directory: no recorded downloads and no stored global speed limit.
$state = Join-Path $Dir 'boltget-state'
New-Item -ItemType Directory -Force -Path $state | Out-Null

# IDM's COM interface (ICIDMLinkTransmitter, see IDM's idmantypeinfo.tlb). On the
# machine this was written on, `IDMan.exe /d <url> /p <dir> /f <file> /n` returned
# at once and never started a download, while this interface works.
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[ComImport, Guid("4BD46AAE-C51F-4BF7-8BC0-2E86E33D1873"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface ICIDMLinkTransmitter {
    void SendLinkToIDM([MarshalAs(UnmanagedType.BStr)] string url, [MarshalAs(UnmanagedType.BStr)] string referer,
        [MarshalAs(UnmanagedType.BStr)] string cookies, [MarshalAs(UnmanagedType.BStr)] string data,
        [MarshalAs(UnmanagedType.BStr)] string user, [MarshalAs(UnmanagedType.BStr)] string password,
        [MarshalAs(UnmanagedType.BStr)] string localPath, [MarshalAs(UnmanagedType.BStr)] string localFileName,
        int flags);
}
public static class IdmLink {
    // Flag 1: no questions or dialogs; start at once.
    public static void Send(string url, string dir, string name) {
        Type t = Type.GetTypeFromCLSID(new Guid("AC746233-E9D3-49CD-862F-068F7B7CCCA4"));
        ICIDMLinkTransmitter o = (ICIDMLinkTransmitter)Activator.CreateInstance(t);
        o.SendLinkToIDM(url, "", "", null, "", "", dir, name, 1);
        Marshal.ReleaseComObject(o);
    }
    public static bool IsClosed(string path) {
        try { using (var f = new System.IO.FileStream(path, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.None)) { return true; } }
        catch (System.IO.IOException) { return false; }
        catch (UnauthorizedAccessException) { return false; }
    }
}
"@

function Get-Targets {
    $lines = & $boltget bench --targets $targetsFile --lab "http://127.0.0.1:$LabPort" --list --json
    if ($LASTEXITCODE -ne 0) { throw "boltget bench --list failed ($LASTEXITCODE)" }
    $all = @{}
    foreach ($l in $lines) { $t = $l | ConvertFrom-Json; $all[$t.name] = $t }
    foreach ($n in $Targets) { if (-not $all.ContainsKey($n)) { throw "no target $n in $targetsFile" } }
    $Targets | ForEach-Object { $all[$_] }
}

function Get-LabStats([switch]$Reset) {
    $q = if ($Reset) { '?reset=1' } else { '' }
    Invoke-RestMethod -UseBasicParsing "http://127.0.0.1:$LabPort/_stats$q"
}

function Remove-Payload([string]$Path) {
    if (Test-Path -LiteralPath $Path) { [System.IO.File]::Delete($Path) }
}

function Invoke-Idm($t, [string]$name) {
    $path = Join-Path $Dir $name
    Remove-Payload $path
    $sw = [Diagnostics.Stopwatch]::StartNew()
    [IdmLink]::Send($t.url, $Dir, $name)
    $deadline = [TimeSpan]::FromSeconds($TimeoutSec)
    while ($sw.Elapsed -lt $deadline) {
        $fi = New-Object IO.FileInfo $path
        if ($fi.Exists -and $fi.Length -eq [int64]$t.size -and [IdmLink]::IsClosed($path)) {
            $sw.Stop()
            return @{ ok = $true; wall_ms = $sw.Elapsed.TotalMilliseconds; internal_ms = $null; max_connections = $null; error = $null; path = $path }
        }
        Start-Sleep -Milliseconds 25
    }
    @{ ok = $false; wall_ms = $sw.Elapsed.TotalMilliseconds; internal_ms = $null; max_connections = $null; error = "timeout after $TimeoutSec s"; path = $path }
}

function Invoke-BoltGet($t, [string]$name, [string]$setting) {
    $path = Join-Path $Dir $name
    Remove-Payload $path
    $log = Join-Path $Dir 'boltget-last.jsonl'
    $argv = @('--state', "`"$state`"", 'get', "`"$($t.url)`"", '-o', "`"$path`"", '--no-state', '--overwrite', '--json')
    if ($setting -ne 'auto') { $argv += @('-c', $setting) }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $boltget -ArgumentList $argv -NoNewWindow -PassThru -RedirectStandardOutput $log
    $null = $p.Handle  # keep the handle so ExitCode is available
    $done = $p.WaitForExit($TimeoutSec * 1000)
    $sw.Stop()
    if (-not $done) { $p.Kill(); return @{ ok = $false; wall_ms = $sw.Elapsed.TotalMilliseconds; error = "timeout after $TimeoutSec s"; path = $path; internal_ms = $null; max_connections = $null } }
    $last = Get-Content $log | Select-Object -Last 1 | ConvertFrom-Json
    $r = @{ ok = ($p.ExitCode -eq 0 -and $last.event -eq 'completed'); wall_ms = $sw.Elapsed.TotalMilliseconds; path = $path; internal_ms = $null; max_connections = $null; error = $null }
    if ($last.event -eq 'completed') { $r.internal_ms = $last.elapsed_ms; $r.max_connections = $last.max_connections }
    else {
        $detail = if ($last.PSObject.Properties['error']) { $last.error.message } else { $last.event }
        $r.error = "exit $($p.ExitCode): $detail"
    }
    $r
}

function Get-Quantile([double[]]$sorted, [double]$p) {
    if ($sorted.Count -eq 0) { return $null }
    if ($sorted.Count -eq 1) { return $sorted[0] }
    $pos = $p * ($sorted.Count - 1); $lo = [math]::Floor($pos); $hi = [math]::Ceiling($pos)
    $sorted[$lo] + ($sorted[$hi] - $sorted[$lo]) * ($pos - $lo)
}

# --- Environment ---------------------------------------------------------------
$idmReg = Get-ItemProperty HKCU:\Software\DownloadManager
$connValues = @{}
foreach ($prop in $idmReg.PSObject.Properties) {
    if ($prop.Name -match 'conn|MaxConn|SpeedLim|Limit') { $connValues[$prop.Name] = "$($prop.Value)" }
}
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$disk = try { Get-PhysicalDisk | Select-Object FriendlyName, MediaType, BusType, Size } catch { "unavailable: $_" }
$net = Get-NetAdapter | Where-Object Status -eq 'Up' | Select-Object Name, InterfaceDescription, LinkSpeed
$info = [ordered]@{
    date            = (Get-Date).ToString('o')
    os              = "$($os.Caption) $($os.Version)"
    cpu             = $cpu.Name.Trim()
    logical_cpus    = $cs.NumberOfLogicalProcessors
    ram_gib         = [math]::Round($cs.TotalPhysicalMemory / 1GB, 1)
    disks           = $disk
    network         = $net
    idm_version     = (Get-Item $IdmPath).VersionInfo.ProductVersion
    idm_registry_version = $idmReg.idmvers
    idm_connection_and_limit_values = $connValues
    idm_note        = 'Settings as installed (not changed by this script). No value matching conn/limit means IDM uses its built-in default.'
    boltget_version = (& $boltget --version)
    boltget_commit  = (git -C $repo rev-parse --short HEAD)
    boltget_dirty   = [bool](git -C $repo status --porcelain)
    targets         = $Targets
    runs            = $Runs
    warmup          = $Warmup
    boltget_settings = $BoltGetSettings
    lab_port        = $LabPort
}
$info | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $Out 'environment.json')
Copy-Item $targetsFile (Join-Path $Out 'targets.toml')

# --- Runs ------------------------------------------------------------------------
$targetList = @(Get-Targets)
$server = Start-Process -FilePath $testserver -ArgumentList @('--port', $LabPort) -NoNewWindow -PassThru
Start-Sleep -Milliseconds 500
$csvPath = Join-Path $Out 'runs.csv'
'target,tool,setting,phase,round,order,ok,wall_ms,internal_ms,max_connections,server_max_concurrent,server_connections,server_requests,server_body_bytes,sha256_ok,error,started' |
    Set-Content -Encoding utf8 $csvPath
$tools = @(@{ tool = 'idm'; setting = 'default' }) + ($BoltGetSettings | ForEach-Object { @{ tool = 'boltget'; setting = $_ } })
$results = New-Object System.Collections.Generic.List[object]
try {
    foreach ($t in $targetList) {
        $lab = $t.url.StartsWith("http://127.0.0.1:$LabPort/")
        Write-Host "== $($t.name)  $($t.url)"
        $rounds = @()
        for ($i = 1; $i -le $Warmup; $i++) { $rounds += , @('warmup', $i) }
        for ($i = 1; $i -le $Runs; $i++) { $rounds += , @('measured', $i) }
        $k = 0
        foreach ($rd in $rounds) {
            for ($j = 0; $j -lt $tools.Count; $j++) {
                $tool = $tools[($j + $k) % $tools.Count]
                $name = "$($t.name).$($tool.tool)-$($tool.setting).bin"
                if ($lab) { $null = Get-LabStats -Reset }
                $started = (Get-Date).ToString('o')
                $r = if ($tool.tool -eq 'idm') { Invoke-Idm $t $name } else { Invoke-BoltGet $t $name $tool.setting }
                $st = if ($lab) { Get-LabStats } else { $null }
                $shaOk = $null
                if ($r.ok -and (Test-Path -LiteralPath $r.path)) {
                    if ($t.sha256) {
                        $shaOk = ((Get-FileHash -Algorithm SHA256 -LiteralPath $r.path).Hash.ToLower() -eq $t.sha256)
                        if (-not $shaOk) { $r.ok = $false; $r.error = 'sha256 mismatch' }
                    }
                }
                Remove-Payload $r.path
                $row = [pscustomobject]@{
                    target = $t.name; tool = $tool.tool; setting = $tool.setting; phase = $rd[0]; round = $rd[1]; order = $j + 1
                    ok = $r.ok; wall_ms = [math]::Round($r.wall_ms, 1); internal_ms = $r.internal_ms; max_connections = $r.max_connections
                    server_max_concurrent = $(if ($st) { $st.max_concurrent }); server_connections = $(if ($st) { $st.connections })
                    server_requests = $(if ($st) { $st.requests }); server_body_bytes = $(if ($st) { $st.body_bytes })
                    sha256_ok = $shaOk; error = $r.error; started = $started
                }
                $results.Add($row)
                ($row | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1) | Add-Content -Encoding utf8 $csvPath
                $msg = "  {0,-8} {1,-8} {2} {3}: {4,8:N0} ms ok={5}" -f $tool.tool, $tool.setting, $rd[0], $rd[1], $r.wall_ms, $r.ok
                if ($st) { $msg += " server_max_conns=$($st.max_concurrent) bytes=$($st.body_bytes)" }
                if ($r.error) { $msg += " error=$($r.error)" }
                Write-Host $msg
                # Let connections close and the disk settle between runs.
                Start-Sleep -Milliseconds 1500
            }
            $k++
        }
    }
}
finally {
    if (-not $server.HasExited) { $server.Kill() }
    Remove-Payload (Join-Path $Dir 'boltget-last.jsonl')
}

# --- Summary ---------------------------------------------------------------------
$summary = foreach ($g in ($results | Where-Object phase -eq 'measured' | Group-Object target, tool, setting)) {
    $okRuns = @($g.Group | Where-Object ok)
    [double[]]$secs = @($okRuns | ForEach-Object { $_.wall_ms / 1000.0 } | Sort-Object)
    $first = $g.Group[0]
    $size = ($targetList | Where-Object name -eq $first.target).size
    $med = Get-Quantile $secs 0.5
    [pscustomobject]@{
        target = $first.target; tool = $first.tool; setting = $first.setting
        runs = $okRuns.Count; failed = $g.Count - $okRuns.Count
        median_s = $med; q1_s = (Get-Quantile $secs 0.25); q3_s = (Get-Quantile $secs 0.75)
        min_s = $(if ($secs.Count) { $secs[0] }); max_s = $(if ($secs.Count) { $secs[-1] })
        median_mib_s = $(if ($med) { [int64]$size / 1MB / $med })
        max_server_connections = $(if (@($g.Group | Where-Object { $null -ne $_.server_max_concurrent }).Count) { ($g.Group | Measure-Object server_max_concurrent -Maximum).Maximum })
        max_reported_connections = $(if (@($g.Group | Where-Object { $null -ne $_.max_connections }).Count) { ($g.Group | Measure-Object max_connections -Maximum).Maximum })
    }
}
$summary | Export-Csv -NoTypeInformation -Encoding utf8 (Join-Path $Out 'summary.csv')
$summary | Format-Table target, tool, setting, runs, failed,
    @{ n = 'median s'; e = { '{0:N2}' -f $_.median_s } },
    @{ n = 'q1-q3 s'; e = { '{0:N2}-{1:N2}' -f $_.q1_s, $_.q3_s } },
    @{ n = 'min-max s'; e = { '{0:N2}-{1:N2}' -f $_.min_s, $_.max_s } },
    @{ n = 'MiB/s'; e = { '{0:N1}' -f $_.median_mib_s } },
    max_server_connections, max_reported_connections -AutoSize | Out-String -Width 200 | Write-Host
Write-Host "Raw data: $Out"
