# Start-up time of Boltget.exe: from launch to the main window becoming
# visible, which the app does only after its first painted frame.
#   powershell -File startup.ps1 -Exe <path> -Runs 5 [-FreshProfile]
param([string]$Exe, [int]$Runs = 5, [switch]$FreshProfile, [string]$Data, [string]$Wv2)
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  public static bool Visible(uint pid) {
    bool found = false;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { var sb = new StringBuilder(64); GetWindowText(h, sb, 64); if (sb.ToString() == "Boltget") { found = true; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
}
'@
$env:BOLTGET_DATA_DIR = $Data
$env:WEBVIEW2_USER_DATA_FOLDER = $Wv2
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
$results = @()
for ($i = 1; $i -le $Runs; $i++) {
  Get-Process Boltget -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 3
  if ($FreshProfile) { Get-ChildItem -Force $Wv2 | Remove-Item -Recurse -Force }
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $Exe -PassThru
  while (-not [W]::Visible([uint32]$p.Id)) {
    if ($sw.ElapsedMilliseconds -gt 15000) { break }
    Start-Sleep -Milliseconds 5
  }
  $ms = $sw.ElapsedMilliseconds
  $results += $ms
  "run $i : $ms ms"
  Start-Sleep -Seconds 3
}
Get-Process Boltget -ErrorAction SilentlyContinue | Stop-Process -Force
$sorted = $results | Sort-Object
"median {0} ms, min {1}, max {2}" -f $sorted[[int][math]::Floor(($sorted.Count - 1) / 2)], $sorted[0], $sorted[-1]
