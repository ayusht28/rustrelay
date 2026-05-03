Set-Location "D:\rustrelay\web"
Write-Host "=== RustRelay Web Deploy ===" -ForegroundColor Cyan

Write-Host "[1/3] Node version:" -ForegroundColor Yellow
node --version

Write-Host "[2/3] Installing Vercel CLI..." -ForegroundColor Yellow
npm install -g vercel

Write-Host "[3/3] Deploying to Vercel (free tier)..." -ForegroundColor Yellow
Write-Host "NOTE: A browser will open for login if needed." -ForegroundColor Green
vercel --prod --yes

Write-Host "=== Done! Copy the URL above ===" -ForegroundColor Cyan
Read-Host "Press Enter to close"
