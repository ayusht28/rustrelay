Set-Location "D:\rustrelay\web"
$logFile = "D:\rustrelay\vercel_login.log"
"" | Out-File $logFile

Write-Host "Running vercel login..." -ForegroundColor Cyan

# Run vercel login and tee to file
& vercel login 2>&1 | ForEach-Object {
    $_ | Out-File $logFile -Append
    Write-Host $_
}

Write-Host ""
Write-Host "Login done. Now deploying..." -ForegroundColor Cyan

$deployLog = "D:\rustrelay\vercel_deploy.log"
"" | Out-File $deployLog

& vercel --prod --yes 2>&1 | ForEach-Object {
    $_ | Out-File $deployLog -Append
    Write-Host $_
}

Write-Host ""
Write-Host "Deploy output saved to: $deployLog" -ForegroundColor Green
Read-Host "Press Enter to close"
