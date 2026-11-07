# Desktop app measurements

The scripts behind [docs/APP-PERFORMANCE.md](../../docs/APP-PERFORMANCE.md): start-up time, idle memory and smoothness with 500 downloads, in the real WebView2. Windows PowerShell 5.1 and Node.js (the version in `.node-version`).

Always use scratch directories: `BOLTGET_DATA_DIR` for the downloads list and `WEBVIEW2_USER_DATA_FOLDER` for the WebView's profile, never your own.

```powershell
pnpm --filter @boltget/desktop installer           # the release build: target\x86_64-pc-windows-msvc\release\Boltget.exe
cargo build --release -p boltget-testserver
Start-Process target\release\boltget-testserver.exe -ArgumentList "--port", "8090"
$exe = "target\x86_64-pc-windows-msvc\release\Boltget.exe"

# Start-up: launch to the first painted window (5 runs with a fresh WebView2 profile, then warm).
powershell -File bench\app\startup.ps1 -Exe $exe -Runs 5 -FreshProfile -Data C:\scratch\data -Wv2 C:\scratch\wv2
powershell -File bench\app\startup.ps1 -Exe $exe -Runs 7 -Data C:\scratch\data -Wv2 C:\scratch\wv2

# 500 downloads and smoothness, through the DevTools protocol.
$env:BOLTGET_DATA_DIR = "C:\scratch\data"; $env:WEBVIEW2_USER_DATA_FOLDER = "C:\scratch\wv2"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
Start-Process $exe
node bench\app\seed.mjs C:\scratch\downloads
node bench\app\frames.mjs

# Idle memory: start without the DevTools port, wait 30 s, then close the window to the tray and wait 30 s.
powershell -File bench\app\mem.ps1
```

`node bench\app\cdp.mjs eval "<js>"` evaluates anything in the page, for example the start-up marks: `performance.getEntriesByType('mark')` (`boltget:shown`, `boltget:list`).
