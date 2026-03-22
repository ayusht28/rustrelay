@echo off
REM ─────────────────────────────────────────────────────────
REM  RustRelay — Windows Setup (No Docker)
REM  Run this AFTER installing PostgreSQL and Memurai/Redis
REM ─────────────────────────────────────────────────────────

echo.
echo === RustRelay Windows Setup ===
echo.

REM Find psql.exe
set PSQL=
if exist "C:\Program Files\PostgreSQL\16\bin\psql.exe" set PSQL="C:\Program Files\PostgreSQL\16\bin\psql.exe"
if exist "C:\Program Files\PostgreSQL\17\bin\psql.exe" set PSQL="C:\Program Files\PostgreSQL\17\bin\psql.exe"
if exist "D:\PostgreSQL\16\bin\psql.exe" set PSQL="D:\PostgreSQL\16\bin\psql.exe"
if exist "D:\PostgreSQL\17\bin\psql.exe" set PSQL="D:\PostgreSQL\17\bin\psql.exe"
if exist "D:\PostgreSQL\bin\psql.exe" set PSQL="D:\PostgreSQL\bin\psql.exe"

if "%PSQL%"=="" (
    echo ERROR: Could not find psql.exe
    echo Please edit this script and set the PSQL path manually.
    pause
    exit /b 1
)

echo Found psql at: %PSQL%
echo.

echo Step 1: Creating database user 'rustrelay'...
echo CREATE USER rustrelay WITH PASSWORD 'password'; | %PSQL% -U postgres
if errorlevel 1 (
    echo Note: User might already exist. Continuing...
)

echo.
echo Step 2: Creating database 'rustrelay'...
echo CREATE DATABASE rustrelay OWNER rustrelay; | %PSQL% -U postgres
if errorlevel 1 (
    echo Note: Database might already exist. Continuing...
)

echo.
echo Step 3: Loading tables and test data...
%PSQL% -U rustrelay -d rustrelay -f migrations\001_initial.sql
if errorlevel 1 (
    echo ERROR: Failed to load migrations. Check your PostgreSQL password.
    pause
    exit /b 1
)

echo.
echo Step 4: Creating .env file...
if not exist .env (
    copy .env.example .env
    echo Created .env from .env.example
    echo IMPORTANT: Open .env and change JWT_SECRET to any random string.
) else (
    echo .env already exists, skipping.
)

echo.
echo ============================================
echo   Setup complete!
echo.
echo   Next steps:
echo   1. Open .env and set JWT_SECRET=mysecret123
echo   2. Make sure Memurai/Redis is running
echo   3. Run: cargo run
echo ============================================
echo.
pause
