# Memory of Boltget.exe and every process under it (its WebView2 browser,
# renderer, GPU and utility processes), in MiB.
#   powershell -File mem.ps1 [-Name Boltget]
param([string]$Name = "Boltget")
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine
$root = $all | Where-Object { $_.Name -eq "$Name.exe" } | Select-Object -First 1
if (-not $root) { throw "$Name is not running" }
$ids = New-Object System.Collections.Generic.List[int]
$ids.Add([int]$root.ProcessId)
$i = 0
while ($i -lt $ids.Count) {
  $parent = $ids[$i]
  foreach ($c in $all | Where-Object { $_.ParentProcessId -eq $parent }) { $ids.Add([int]$c.ProcessId) }
  $i++
}
$rows = foreach ($id in $ids) {
  $p = Get-Process -Id $id -ErrorAction SilentlyContinue
  if (-not $p) { continue }
  $cmd = ($all | Where-Object { $_.ProcessId -eq $id }).CommandLine
  $kind = if ($cmd -match '--type=([a-z-]+)') { $Matches[1] } elseif ($p.ProcessName -eq 'msedgewebview2') { 'browser' } else { 'app' }
  [pscustomobject]@{
    Process = $p.ProcessName; Kind = $kind; Id = $id
    WorkingSetMiB = [math]::Round($p.WorkingSet64 / 1MB, 1)
    PrivateMiB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
  }
}
$rows | Format-Table -AutoSize | Out-String
"TOTAL working set {0:N1} MiB, private {1:N1} MiB, processes {2}" -f (($rows | Measure-Object WorkingSetMiB -Sum).Sum), (($rows | Measure-Object PrivateMiB -Sum).Sum), $rows.Count
