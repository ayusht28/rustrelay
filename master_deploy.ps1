# master_deploy.ps1
# End-to-end: git init -> GitHub -> Render -> Vercel frontend update
# Uses gh CLI device flow for GitHub auth (opens browser automatically)
# Plain ASCII only

$ErrorActionPreference = "Continue"
$logFile = "D:\rustrelay\master_deploy.log"
"=== RustRelay Master Deploy ===" | Out-File $logFile -Encoding utf8

function Log($msg) {
    $ts = (Get-Date).ToString("HH:mm:ss")
    "[$ts] $msg" | Out-File $logFile -Append -Encoding utf8
    Write-Host "[$ts] $msg"
}

function Fail($msg) {
    Log "FATAL: $msg"
    Log "See full log: $logFile"
    Read-Host "Press Enter to close"
    exit 1
}

function RefreshPath {
    $m = [System.Environment]::GetEnvironmentVariable("PATH", "Machine")
    $u = [System.Environment]::GetEnvironmentVariable("PATH", "User")
    $env:PATH = "$m;$u;$env:USERPROFILE\.fly\bin;$env:LOCALAPPDATA\Programs\gh\bin"
}

function ToolExists($name) {
    return ($null -ne (Get-Command $name -ErrorAction SilentlyContinue))
}

# ============================================================================
# STEP 1: Git init and commit
# ============================================================================
Log "STEP 1: Setting up git repo..."
Set-Location "D:\rustrelay\rustrelay"

if (-not (ToolExists "git")) {
    Fail "Git is not installed. Install from https://git-scm.com"
}

if (-not (Test-Path ".git")) {
    Log "  Running git init..."
    git init 2>&1 | Out-Null

    git config user.email "ayushthakare122005@gmail.com" 2>&1 | Out-Null
    git config user.name "Ayush Thakare" 2>&1 | Out-Null

    Log "  Staging all files..."
    git add . 2>&1 | Out-Null

    Log "  Committing..."
    $out = git commit -m "Initial commit: RustRelay - high-performance Rust messaging backend" 2>&1
    $code = $LASTEXITCODE
    Log "  $out"
    if ($code -ne 0) { Fail "git commit failed (exit $code)" }
    Log "  Committed."
} else {
    Log "  Git repo already exists. Staging any changes..."
    git add . 2>&1 | Out-Null
    git diff --staged --quiet 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        git commit -m "Update: Redis optional + Render config" 2>&1 | Out-Null
        Log "  Changes committed."
    } else {
        Log "  No new changes to commit."
    }
}

# ============================================================================
# STEP 2: Install GitHub CLI (gh)
# ============================================================================
Log ""
Log "STEP 2: Checking GitHub CLI..."

if (-not (ToolExists "gh")) {
    Log "  Installing gh via winget (silent)..."
    $out = winget install --id GitHub.cli --silent --accept-package-agreements --accept-source-agreements 2>&1
    $code = $LASTEXITCODE
    foreach ($line in $out) { Log "    $line" }
    RefreshPath

    if (-not (ToolExists "gh")) {
        Log "  winget install may have succeeded but gh not on PATH yet."
        Log "  Trying common install paths..."
        $ghPaths = @(
            "$env:ProgramFiles\GitHub CLI\gh.exe",
            "$env:LOCALAPPDATA\Programs\GitHub CLI\gh.exe",
            "C:\Program Files\GitHub CLI\gh.exe"
        )
        foreach ($p in $ghPaths) {
            if (Test-Path $p) {
                $dir = Split-Path $p
                $env:PATH = "$dir;$env:PATH"
                Log "  Found gh at: $p"
                break
            }
        }
        if (-not (ToolExists "gh")) {
            Fail "gh CLI not found after install. Please install from https://cli.github.com and re-run."
        }
    }
}
$ghVer = gh --version 2>&1 | Select-Object -First 1
Log "  gh: $ghVer"

# ============================================================================
# STEP 3: GitHub Authentication
# ============================================================================
Log ""
Log "STEP 3: GitHub authentication..."

$authOut = gh auth status 2>&1 | Out-String
$authCode = $LASTEXITCODE

if ($authCode -ne 0) {
    Log "  Not authenticated. Starting GitHub device flow..."
    Log "  A browser window will open. If prompted, click Authorize."
    Log ""
    # gh auth login opens browser automatically, waits for auth to complete
    gh auth login --git-protocol https --web
    $code = $LASTEXITCODE
    if ($code -ne 0) { Fail "GitHub auth failed (exit $code). Re-run and complete browser auth." }
    Log "  GitHub auth complete."
} else {
    Log "  Already authenticated with GitHub."
}

$ghUser = gh api user --jq .login 2>&1
$ghCode = $LASTEXITCODE
if ($ghCode -ne 0) { Fail "Could not get GitHub username. Auth may have failed." }
Log "  GitHub user: $ghUser"

# ============================================================================
# STEP 4: Create GitHub repo and push
# ============================================================================
Log ""
Log "STEP 4: Creating GitHub repo and pushing code..."

$repoName = "rustrelay"
$repoFullName = "$ghUser/$repoName"

# Check if repo already exists
$repoCheck = gh repo view $repoFullName 2>&1
$repoCode = $LASTEXITCODE

if ($repoCode -ne 0) {
    Log "  Creating public repo $repoFullName..."
    $out = gh repo create $repoName --public --description "RustRelay: high-performance real-time messaging backend in Rust" 2>&1
    $code = $LASTEXITCODE
    foreach ($line in $out) { Log "  $line" }
    if ($code -ne 0) { Fail "Could not create GitHub repo (exit $code)." }
    Log "  Repo created."
} else {
    Log "  Repo $repoFullName already exists."
}

# Set remote
git remote remove origin 2>&1 | Out-Null
$remoteUrl = "https://github.com/$repoFullName.git"
git remote add origin $remoteUrl 2>&1 | Out-Null
Log "  Remote: $remoteUrl"

# Push
Log "  Pushing to GitHub..."
$out = git push -u origin main 2>&1
$pushCode = $LASTEXITCODE
foreach ($line in $out) { Log "  $line" }

if ($pushCode -ne 0) {
    Log "  main failed, trying master..."
    $out = git push -u origin master 2>&1
    $pushCode = $LASTEXITCODE
    foreach ($line in $out) { Log "  $line" }
}

if ($pushCode -ne 0) { Fail "git push failed (exit $pushCode). Check the log." }
Log "  Code is on GitHub: https://github.com/$repoFullName"

# ============================================================================
# STEP 5: Create render.yaml if missing
# ============================================================================
Log ""
Log "STEP 5: Ensuring render.yaml exists..."

$renderYaml = "D:\rustrelay\rustrelay\render.yaml"
if (-not (Test-Path $renderYaml)) {
    Log "  Writing render.yaml..."
    $yaml = @"
services:
  - type: web
    name: rustrelay
    env: docker
    dockerfilePath: ./Dockerfile
    plan: free
    envVars:
      - key: HOST
        value: 0.0.0.0
      - key: PORT
        value: "10000"
      - key: NODE_ID
        value: node-render-1
      - key: REDIS_URL
        value: ""
      - key: JWT_SECRET
        generateValue: true
      - key: RUST_LOG
        value: rustrelay=info
      - key: HEARTBEAT_INTERVAL_SECS
        value: "30"
      - key: HEARTBEAT_TIMEOUT_SECS
        value: "3600"
      - key: PRESENCE_OFFLINE_DEBOUNCE_SECS
        value: "5"
      - key: READSTATE_FLUSH_INTERVAL_SECS
        value: "5"
      - key: DATABASE_URL
        fromDatabase:
          name: rustrelay-db
          property: connectionString
databases:
  - name: rustrelay-db
    plan: free
"@
    $yaml | Out-File $renderYaml -Encoding utf8
    Log "  render.yaml written."

    # Commit and push the new file
    Set-Location "D:\rustrelay\rustrelay"
    git add render.yaml 2>&1 | Out-Null
    git commit -m "Add render.yaml blueprint" 2>&1 | Out-Null
    git push origin HEAD 2>&1 | Out-Null
    Log "  render.yaml pushed to GitHub."
} else {
    Log "  render.yaml already exists."
    # Make sure it's pushed
    git add render.yaml 2>&1 | Out-Null
    git diff --staged --quiet 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        git commit -m "Update render.yaml" 2>&1 | Out-Null
        git push origin HEAD 2>&1 | Out-Null
    }
}

# ============================================================================
# STEP 6: Deploy to Render via API
# ============================================================================
Log ""
Log "STEP 6: Setting up Render deployment..."

# Check if render CLI or API token is available
$renderToken = $env:RENDER_API_TOKEN
$renderServiceUrl = ""

if ([string]::IsNullOrWhiteSpace($renderToken)) {
    Log "  No RENDER_API_TOKEN in env. Attempting Render signup via GitHub OAuth..."
    Log ""
    Log "  Opening Render in browser. Sign in with GitHub (one click)."
    Log "  After signing in, come back here and press Enter."
    Log ""

    # Open Render signup via GitHub OAuth
    $renderSignupUrl = "https://dashboard.render.com/register?next=https%3A%2F%2Fdashboard.render.com%2Fnew%2Fweb-service%3Frepo%3Dhttps%3A%2F%2Fgithub.com%2F${ghUser}%2F${repoName}"

    # Write the URL to a file so we can open it
    $urlFile = "D:\rustrelay\render_signup_url.txt"
    "RENDER SIGNUP URL (copy and paste into browser if it doesn't open):" | Out-File $urlFile -Encoding utf8
    "" | Out-File $urlFile -Append
    $renderSignupUrl | Out-File $urlFile -Append
    "" | Out-File $urlFile -Append
    "After signing in and creating the service, copy the service URL and paste it below." | Out-File $urlFile -Append

    # Open the URL in the default browser
    Start-Process $renderSignupUrl

    Log "  Browser opened to Render. Sign in with GitHub, then connect your rustrelay repo."
    Log "  The render.yaml in your repo auto-configures everything."
    Log ""

    $renderServiceUrl = Read-Host "  Paste your Render service URL when ready (e.g. https://rustrelay-xxxx.onrender.com)"
    if ([string]::IsNullOrWhiteSpace($renderServiceUrl)) {
        Log "  No URL provided. Saving GitHub URL and exiting."
        Log "  Deploy to Render manually: https://dashboard.render.com/new/web-service"
        Log "  Connect repo: https://github.com/$repoFullName"
        Read-Host "Press Enter to close"
        exit 0
    }
    $renderServiceUrl = $renderServiceUrl.Trim().TrimEnd("/")
} else {
    Log "  Using RENDER_API_TOKEN from environment."
    # Use Render API to create service
    $headers = @{
        "Authorization" = "Bearer $renderToken"
        "Content-Type"  = "application/json"
    }
    # Get owner ID
    $owner = Invoke-RestMethod -Uri "https://api.render.com/v1/owners?limit=1" -Headers $headers
    $ownerId = $owner.owner.id

    $body = @{
        type = "web_service"
        name = "rustrelay"
        ownerId = $ownerId
        repo = "https://github.com/$repoFullName"
        branch = "main"
        envVars = @(
            @{ key = "PORT"; value = "10000" }
            @{ key = "HOST"; value = "0.0.0.0" }
            @{ key = "REDIS_URL"; value = "" }
        )
        serviceDetails = @{
            env = "docker"
            plan = "free"
        }
    } | ConvertTo-Json -Depth 5

    $svc = Invoke-RestMethod -Uri "https://api.render.com/v1/services" -Method POST -Headers $headers -Body $body
    $renderServiceUrl = "https://" + $svc.service.serviceDetails.url
    Log "  Render service created: $renderServiceUrl"
}

# ============================================================================
# STEP 7: Update Vercel frontend with Render URL
# ============================================================================
Log ""
Log "STEP 7: Updating Vercel frontend with Render WebSocket URL..."

$wsUrl = $renderServiceUrl -replace "^https://", "wss://"
if (-not $wsUrl.EndsWith("/ws")) {
    $wsUrl = "$wsUrl/ws"
}
Log "  WebSocket URL: $wsUrl"

$htmlPath = "D:\rustrelay\web\index.html"
$content = Get-Content $htmlPath -Raw -Encoding utf8

# Replace the ws-url input default value (handles localhost or any previous value)
$content = $content -replace 'value="ws[^"]*"', "value=`"$wsUrl`""
$content = $content -replace 'placeholder="[^"]*"(\s*/>)', "placeholder=`"$wsUrl`"`$1"

Set-Content $htmlPath $content -NoNewline -Encoding utf8
Log "  index.html updated."

# ============================================================================
# STEP 8: Redeploy Vercel
# ============================================================================
Log ""
Log "STEP 8: Redeploying Vercel frontend..."

if (ToolExists "vercel") {
    Set-Location "D:\rustrelay\web"
    vercel --prod --yes 2>&1 | ForEach-Object { Log "  $_" }
    $vercelCode = $LASTEXITCODE
    if ($vercelCode -eq 0) {
        Log "  Vercel redeployed successfully."
    } else {
        Log "  Vercel redeploy failed (exit $vercelCode). Run run_redeploy.vbs manually."
    }
} else {
    Log "  vercel CLI not found. Run run_redeploy.vbs to redeploy manually."
}

# ============================================================================
# DONE
# ============================================================================
Log ""
Log "============================================"
Log "DEPLOYMENT SUMMARY"
Log "============================================"
Log ""
Log "  GitHub repo:    https://github.com/$repoFullName"
Log "  Render backend: $renderServiceUrl"
Log "  WebSocket:      $wsUrl"
Log "  Vercel frontend: https://web-sigma-three-51.vercel.app"
Log ""
Log "  NOTE: Render free tier spins down after 15 min of inactivity."
Log "  First request after spin-down takes ~30s to start."
Log ""
Log "Full log: $logFile"

Read-Host "Press Enter to close"
