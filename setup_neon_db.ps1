# RustRelay - Run database migrations on Neon (free PostgreSQL)
# Usage: After creating a Neon project, paste the connection string when prompted.

Write-Host ""
Write-Host "=== RustRelay Database Setup ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "This script runs the database migrations on your PostgreSQL instance."
Write-Host "If you need a FREE PostgreSQL database, go to: https://neon.tech"
Write-Host "  1. Sign up (free)"
Write-Host "  2. Create a project"
Write-Host "  3. Copy the connection string (looks like: postgres://user:pass@host.neon.tech/neondb?sslmode=require)"
Write-Host ""

$dbUrl = Read-Host "Paste your DATABASE_URL here"
if ([string]::IsNullOrWhiteSpace($dbUrl)) {
    Write-Host "No URL provided. Exiting." -ForegroundColor Red
    Read-Host "Press Enter to close"
    exit 1
}

Write-Host ""
Write-Host "Checking psql..." -ForegroundColor Yellow

# Try to find psql
$psqlPaths = @(
    "C:\Program Files\PostgreSQL\16\bin\psql.exe",
    "C:\Program Files\PostgreSQL\15\bin\psql.exe",
    "C:\Program Files\PostgreSQL\14\bin\psql.exe",
    "psql"
)
$psql = $null
foreach ($p in $psqlPaths) {
    try {
        $result = & $p --version 2>&1
        if ($LASTEXITCODE -eq 0) {
            $psql = $p
            break
        }
    } catch {}
}

if (-not $psql) {
    Write-Host "psql not found. Trying to use the migration SQL directly via PowerShell..." -ForegroundColor Yellow
    # Fall back to using the .NET Npgsql library
    Write-Host "Install psql from: https://www.enterprisedb.com/downloads/postgres-postgresql-downloads" -ForegroundColor Red
    Write-Host "Then re-run this script." -ForegroundColor Red
    Read-Host "Press Enter to close"
    exit 1
}

Write-Host "Found psql: $psql" -ForegroundColor Green
Write-Host ""
Write-Host "Running migrations..." -ForegroundColor Yellow

$env:PGPASSWORD = ""  # psql will use the URL
& $psql $dbUrl -f "D:\rustrelay\rustrelay\migrations\001_initial.sql" 2>&1

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "=== Migrations complete! ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Save this DATABASE_URL — you'll need it for Fly.io:" -ForegroundColor Cyan
    Write-Host $dbUrl -ForegroundColor White
    Write-Host ""
    Write-Host "Set it on Fly.io with:"
    Write-Host "  flyctl secrets set DATABASE_URL=`"$dbUrl`" --app rustrelay-demo" -ForegroundColor Gray
} else {
    Write-Host ""
    Write-Host "Migration failed. Check the error above." -ForegroundColor Red
}

Read-Host "Press Enter to close"
