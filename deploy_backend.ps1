# deploy_backend.ps1
# Installs flyctl + gh, pushes to GitHub, deploys RustRelay to Fly.io.
# Plain ASCII only.
#
# IMPORTANT PowerShell note used throughout this script:
#   Piping to ForEach-Object resets $LASTEXITCODE to 0.
#   So we always capture output into a variable FIRST, then check
#   $LASTEXITCODE, then log. Pattern:
#       $out = some-command 2>&1
#       $code = $LASTEXITCODE
#       $out | ForEach-Object { Log $_ }
#       if ($code -ne 0) { Fail "..." }

Set-Location "D:\rustrelay\rustrelay"

$logFile = "D:\rustrelay\backend_deploy.log"
"=== RustRelay Backend Deploy ===" | Out-File $logFile -Encoding utf8

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

function Log($msg) {
    $msg | Out-File $logFile -Append -Encoding utf8
    Write-Host $msg
}

function Fail($msg) {
    Log ""
    Log "ERROR: $msg"
    Log ""
    Log "Deployment stopped. Fix the issue above, then re-run the script."
    Read-Host "Press Enter to close"
    exit 1
}

function RunCmd {
    # Run a command-line tool with arguments, capture ALL output,
    # log it, and RETURN the real exit code.
    # Usage: $code = RunCmd "git" "init"
    #    or: $code = RunCmd "flyctl" "apps" "list"
    param([string]$Tool, [string[]]$Args)
    $out = & $Tool @Args 2>&1
    $code = $LASTEXITCODE
    foreach ($line in $out) { Log ([string]$line) }
    return $code
}

function RefreshPath {
    $machinePath = [System.Environment]::GetEnvironmentVariable("PATH", "Machine")
    $userPath    = [System.Environment]::GetEnvironmentVariable("PATH", "User")
    $flyPath     = "$env:USERPROFILE\.fly\bin"
    $env:PATH    = "$machinePath;$userPath;$flyPath"
}

function ToolExists($name) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    return ($null -ne $cmd)
}

# ---------------------------------------------------------------------------
# Step 1: Require Git
# ---------------------------------------------------------------------------
Log "Step 1: Checking Git..."
if (-not (ToolExists "git")) {
    Fail "Git is not installed. Download from https://git-scm.com and re-run."
}
$out = & git --version 2>&1
Log "  $out"

# ---------------------------------------------------------------------------
# Step 2: Init repo and commit
# ---------------------------------------------------------------------------
Log ""
Log "Step 2: Initialising git repo..."

if (-not (Test-Path ".git")) {
    $code = RunCmd "git" "init"
    if ($code -ne 0) { Fail "git init failed (exit $code)." }

    # Configure a default identity if git has none (fresh machine)
    & git config user.email "deploy@rustrelay.local" 2>&1 | Out-Null
    & git config user.name  "RustRelay Deploy"       2>&1 | Out-Null

    $code = RunCmd "git" "add", "."
    if ($code -ne 0) { Fail "git add failed (exit $code)." }

    $code = RunCmd "git" "commit", "-m", "Initial commit: RustRelay backend"
    if ($code -ne 0) { Fail "git commit failed (exit $code)." }

    Log "  Repo initialised."
} else {
    Log "  Repo already exists. Staging changes..."
    RunCmd "git" "add", "." | Out-Null

    # Check if there is anything staged
    & git diff --staged --quiet 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        $code = RunCmd "git" "commit", "-m", "Update: Redis optional + Fly.io config"
        if ($code -ne 0) { Fail "git commit failed (exit $code)." }
        Log "  Changes committed."
    } else {
        Log "  Nothing new to commit."
    }
}

# ---------------------------------------------------------------------------
# Step 3: Install flyctl
# ---------------------------------------------------------------------------
Log ""
Log "Step 3: Checking flyctl..."

if (-not (ToolExists "flyctl")) {
    Log "  flyctl not found. Installing via official install script..."
    # The official Windows installer from fly.io
    Invoke-WebRequest -Uri "https://fly.io/install.ps1" -UseBasicParsing | Invoke-Expression
    RefreshPath

    if (-not (ToolExists "flyctl")) {
        Fail "flyctl still not found after install. Try restarting PowerShell and re-running the script."
    }
    Log "  flyctl installed."
} else {
    Log "  flyctl already installed."
}

$out = & flyctl version 2>&1
Log "  $out"

# ---------------------------------------------------------------------------
# Step 4: Log in to Fly.io
# ---------------------------------------------------------------------------
Log ""
Log "Step 4: Fly.io login..."

# Check if already authenticated
$out = & flyctl auth whoami 2>&1
$code = $LASTEXITCODE
if ($code -ne 0) {
    Log "  Not logged in. A browser window will open for Fly.io sign-in."
    Log "  Sign up at https://fly.io if you do not have an account (free)."
    Log ""
    # Run interactively so the user can complete the browser flow
    flyctl auth login
    $code = $LASTEXITCODE
    if ($code -ne 0) { Fail "Fly.io login failed (exit $code). Re-run the script and complete the browser auth." }
    $out = & flyctl auth whoami 2>&1
}
Log "  Logged in as: $out"
Read-Host "  (Press Enter to continue after completing browser auth)"

# ---------------------------------------------------------------------------
# Step 5: Install GitHub CLI (gh)
# ---------------------------------------------------------------------------
Log ""
Log "Step 5: Checking GitHub CLI..."

if (-not (ToolExists "gh")) {
    Log "  gh not found. Installing via winget..."
    $code = RunCmd "winget" "install", "--id", "GitHub.cli", "--silent", "--accept-package-agreements", "--accept-source-agreements"
    RefreshPath

    if (-not (ToolExists "gh")) {
        Fail "gh CLI still not found after install. Try restarting PowerShell and re-running, or install manually from https://cli.github.com"
    }
    Log "  gh installed."
} else {
    Log "  gh already installed."
}

$out = & gh --version 2>&1
Log "  $out"

# ---------------------------------------------------------------------------
# Step 6: Log in to GitHub
# ---------------------------------------------------------------------------
Log ""
Log "Step 6: GitHub login..."

$out = & gh auth status 2>&1
$code = $LASTEXITCODE
if ($code -ne 0) {
    Log "  Not logged in. A browser window will open for GitHub sign-in."
    # Run interactively
    gh auth login --web
    $code = $LASTEXITCODE
    if ($code -ne 0) { Fail "GitHub login failed (exit $code). Re-run and complete the browser auth." }
}
Log "  GitHub: authenticated."

# ---------------------------------------------------------------------------
# Step 7: Create GitHub repo and push
# ---------------------------------------------------------------------------
Log ""
Log "Step 7: Pushing to GitHub..."

$remoteUrl = "https://github.com/ayushthakare122005/rustrelay.git"

# Check if the remote is already set
$out = & git remote get-url origin 2>&1
$code = $LASTEXITCODE
if ($code -ne 0) {
    Log "  Adding remote origin: $remoteUrl"
    $code = RunCmd "git" "remote", "add", "origin", $remoteUrl
    if ($code -ne 0) { Fail "Could not add git remote (exit $code)." }
} else {
    Log "  Remote origin already set: $out"
}

# Create the GitHub repo if it does not exist yet
$out = & gh repo view ayushthakare122005/rustrelay 2>&1
$code = $LASTEXITCODE
if ($code -ne 0) {
    Log "  Creating GitHub repo ayushthakare122005/rustrelay..."
    $code = RunCmd "gh" "repo", "create", "rustrelay", "--public", "--description", "RustRelay: high-performance real-time messaging backend in Rust"
    if ($code -ne 0) { Fail "Could not create GitHub repo (exit $code). Check that you are logged in as ayushthakare122005." }
} else {
    Log "  GitHub repo already exists."
}

# Push (try main first, fall back to master)
Log "  Pushing code..."
$out = & git push -u origin main 2>&1
$code = $LASTEXITCODE
foreach ($line in $out) { Log ([string]$line) }
if ($code -ne 0) {
    Log "  main branch push failed, trying master..."
    $out = & git push -u origin master 2>&1
    $code = $LASTEXITCODE
    foreach ($line in $out) { Log ([string]$line) }
    if ($code -ne 0) { Fail "git push failed (exit $code). Is the GitHub repo accessible?" }
}
Log "  Code pushed to GitHub."

# ---------------------------------------------------------------------------
# Step 8: Create Fly.io app
# ---------------------------------------------------------------------------
Log ""
Log "Step 8: Creating Fly.io app..."

$out = & flyctl apps list 2>&1
$code = $LASTEXITCODE
if ($code -ne 0) { Fail "Could not list Fly.io apps (exit $code). Is flyctl logged in?" }

if (($out | Out-String) -notmatch "rustrelay-demo") {
    Log "  Creating app rustrelay-demo..."
    $code = RunCmd "flyctl" "apps", "create", "rustrelay-demo"
    if ($code -ne 0) {
        Fail "Could not create Fly.io app (exit $code). If the name is taken, edit fly.toml, change the 'app' value to a unique name, and re-run."
    }
    Log "  App created."
} else {
    Log "  App rustrelay-demo already exists."
}

# ---------------------------------------------------------------------------
# Step 9: Create Fly.io PostgreSQL cluster
# ---------------------------------------------------------------------------
Log ""
Log "Step 9: Setting up PostgreSQL..."

$out = & flyctl postgres list 2>&1
$code = $LASTEXITCODE

if (($out | Out-String) -notmatch "rustrelay-db") {
    Log "  Creating PostgreSQL cluster rustrelay-db (this may take a minute)..."
    $code = RunCmd "flyctl" "postgres", "create", "--name", "rustrelay-db", "--region", "iad", "--initial-cluster-size", "1", "--vm-size", "shared-cpu-1x", "--volume-size", "1"
    if ($code -ne 0) { Fail "Could not create Fly.io PostgreSQL cluster (exit $code)." }

    Log "  Attaching PostgreSQL to rustrelay-demo..."
    $code = RunCmd "flyctl" "postgres", "attach", "rustrelay-db", "--app", "rustrelay-demo"
    if ($code -ne 0) { Fail "Could not attach PostgreSQL to app (exit $code)." }
    Log "  PostgreSQL ready. DATABASE_URL has been set as a secret automatically."
} else {
    Log "  PostgreSQL cluster rustrelay-db already exists."
    # Attach in case it was created but not yet attached
    $secrets = & flyctl secrets list --app rustrelay-demo 2>&1
    if (($secrets | Out-String) -notmatch "DATABASE_URL") {
        Log "  Attaching PostgreSQL to app..."
        $code = RunCmd "flyctl" "postgres", "attach", "rustrelay-db", "--app", "rustrelay-demo"
        if ($code -ne 0) { Fail "Could not attach PostgreSQL to app (exit $code)." }
    } else {
        Log "  DATABASE_URL already set."
    }
}

# ---------------------------------------------------------------------------
# Step 10: Set remaining secrets
# ---------------------------------------------------------------------------
Log ""
Log "Step 10: Setting JWT secret..."
$jwtSecret = [System.Guid]::NewGuid().ToString("N") + [System.Guid]::NewGuid().ToString("N")
$code = RunCmd "flyctl" "secrets", "set", "JWT_SECRET=$jwtSecret", "--app", "rustrelay-demo"
if ($code -ne 0) { Fail "Could not set JWT_SECRET (exit $code)." }
Log "  Secrets set."

# ---------------------------------------------------------------------------
# Step 11: Deploy
# ---------------------------------------------------------------------------
Log ""
Log "Step 11: Deploying to Fly.io..."
Log "  Building Docker image and deploying. First build takes about 5 minutes."
Log ""

# Run deploy interactively so build output is visible in real time
flyctl deploy --app rustrelay-demo
$deployCode = $LASTEXITCODE

Log ""
if ($deployCode -eq 0) {
    Log "=== DEPLOYMENT COMPLETE ==="
    Log ""
    Log "  Backend:   https://rustrelay-demo.fly.dev"
    Log "  WebSocket: wss://rustrelay-demo.fly.dev/ws"
    Log ""
    Log "Next step:"
    Log "  Double-click run_update_frontend.vbs to update the Vercel site."
    Log ""
    Log "Full log saved to: D:\rustrelay\backend_deploy.log"
} else {
    Log "=== DEPLOYMENT FAILED (exit $deployCode) ==="
    Log ""
    Log "Check the output above for the error."
    Log ""
    Log "Common fixes:"
    Log "  App name taken  -> edit fly.toml, change 'app = ...' to a unique name"
    Log "  Build error     -> run: flyctl logs --app rustrelay-demo"
    Log "  No DATABASE_URL -> run: flyctl postgres attach rustrelay-db --app rustrelay-demo"
}

Read-Host "Press Enter to close"
