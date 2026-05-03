Set-Location "D:\rustrelay\web"
$logFile = "D:\rustrelay\vercel_deploy.log"
"Starting deployment..." | Out-File $logFile

Write-Host "Deploying to Vercel..." -ForegroundColor Cyan

& vercel --prod --yes 2>&1 | ForEach-Object {
    $_ | Out-File $logFile -Append
    Write-Host $_
}

Write-Host ""
Write-Host "Done! Check D:\rustrelay\vercel_deploy.log for the URL." -ForegroundColor Green
Read-Host "Press Enter to close"
