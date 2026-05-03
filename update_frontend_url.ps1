# Updates the Vercel frontend with your live Fly.io backend URL,
# then redeploys to Vercel.

$logFile = "D:\rustrelay\frontend_update.log"
"=== Frontend Update Log ===" | Out-File $logFile -Encoding utf8

function Log($msg) {
    $msg | Out-File $logFile -Append -Encoding utf8
    Write-Host $msg
}

Write-Host ""
Write-Host "=== Update RustRelay Frontend ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "This updates the WebSocket demo URL to point to your live backend."
Write-Host ""

# Ask for the Fly.io app name
$appName = Read-Host "Enter your Fly.io app name (press Enter for 'rustrelay-demo')"
if ([string]::IsNullOrWhiteSpace($appName)) {
    $appName = "rustrelay-demo"
}

$wsUrl = "wss://$appName.fly.dev/ws"
Write-Host ""
Write-Host "Will update WebSocket URL to: $wsUrl" -ForegroundColor Green
Write-Host ""
$confirm = Read-Host "Proceed? (y/n)"
if ($confirm -ne "y" -and $confirm -ne "Y") {
    Write-Host "Cancelled."
    exit 0
}

Log "Updating URL to: $wsUrl"

# Read index.html
$htmlPath = "D:\rustrelay\web\index.html"
if (-not (Test-Path $htmlPath)) {
    Log "ERROR: index.html not found at $htmlPath"
    Read-Host "Press Enter to close"
    exit 1
}

$content = Get-Content $htmlPath -Raw -Encoding utf8

# Replace the ws-url input default value
$content = $content -replace 'value="ws://localhost:8080/ws"', "value=`"$wsUrl`""

# Replace the placeholder text
$content = $content -replace 'placeholder="[^"]*8080[^"]*"', "placeholder=`"$wsUrl`""

# Write back
Set-Content $htmlPath $content -NoNewline -Encoding utf8
Log "Updated index.html with new WebSocket URL."

Log ""
Log "Redeploying to Vercel..."
Set-Location "D:\rustrelay\web"
vercel --prod --yes 2>&1 | ForEach-Object { Log $_ }

if ($LASTEXITCODE -eq 0) {
    Log ""
    Log "=== SUCCESS ==="
    Log "Frontend redeployed to Vercel."
    Log "WebSocket demo now points to: $wsUrl"
    Log ""
    Log "Live URLs:"
    Log "  Frontend:  https://web-sigma-three-51.vercel.app"
    Log "  Backend:   https://$appName.fly.dev"
    Log "  WebSocket: $wsUrl"
} else {
    Log ""
    Log "=== Vercel redeploy failed ==="
    Log "Check the output above. You may need to run 'vercel login' first."
}

Write-Host ""
Read-Host "Press Enter to close"
