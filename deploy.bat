@echo off
title RustRelay — Deploy to Vercel
color 0B
echo.
echo  ==================================================
echo   RustRelay Web — Deploying to Vercel (free tier)
echo  ==================================================
echo.

cd /d D:\rustrelay\web

echo [1/3] Checking Node.js...
node --version
if errorlevel 1 (
  echo ERROR: Node.js not found. Please install from nodejs.org
  pause
  exit /b 1
)

echo.
echo [2/3] Installing Vercel CLI (if needed)...
call npm install -g vercel 2>&1
echo.

echo [3/3] Logging in to Vercel...
echo  Getting authentication URL...
echo.
vercel login 2>&1 | powershell -Command "$input | Tee-Object -FilePath 'D:\rustrelay\vercel_login.log'"
echo.
echo [4/4] Deploying to Vercel...
echo.
call vercel --prod --yes 2>&1 | powershell -Command "$input | Tee-Object -FilePath 'D:\rustrelay\vercel_deploy.log'"

echo.
echo  =================================
echo   Deployment complete!
echo   Copy the URL shown above ^(https://...vercel.app^)
echo  =================================
echo.
pause
