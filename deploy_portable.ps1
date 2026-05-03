# deploy_portable.ps1
# Downloads gh CLI as a portable zip (no admin/UAC required)
# Then authenticates with GitHub, creates repo, pushes code, deploys to Render
# Plain ASCII only

$logFile = "D:\rustrelay\portable_deploy.log"
"=== RustRelay Portable Deploy ===" | Out-File $logFile -Encoding utf8

function Log($msg) {
    $ts = (Get-Date).ToString("HH:mm:ss")
    "[$ts] $msg" | Out-File $logFile -Append -Encoding utf8
    Write-Host "[$ts] $msg"
}

function Fail($msg) {
    Log "FATAL: $msg"
    Read-Host "Press Enter to close"
    exit 1
}

# ============================================================================
# STEP 1: Git init and commit
# ============================================================================
Log "STEP 1: Git setup..."
Set-Location "D:\rustrelay\rustrelay"

$gitCmd = Get-Command git -ErrorAction SilentlyContinue
if (-not $gitCmd) { Fail "Git is not installed." }

if (-not (Test-Path ".git")) {
    git init 2>&1 | Out-Null
    git config user.email "ayushthakare122005@gmail.com" 2>&1 | Out-Null
    git config user.name "Ayush Thakare" 2>&1 | Out-Null
    git add . 2>&1 | Out-Null
    $out = git commit -m "Initial commit: RustRelay backend" 2>&1
    $code = $LASTEXITCODE
    Log "  $out"
    if ($code -ne 0) { Fail "git commit failed." }
} else {
    git config user.email "ayushthakare122005@gmail.com" 2>&1 | Out-Null
    git config user.name "Ayush Thakare" 2>&1 | Out-Null
    git add . 2>&1 | Out-Null
    git diff --staged --quiet 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        git commit -m "Update: Redis optional + Render config + render.yaml" 2>&1 | Out-Null
        Log "  Changes committed."
    } else {
        Log "  No new changes."
    }
}
Log "  Git repo ready."

# ============================================================================
# STEP 2: Get gh CLI portable (no admin/UAC needed)
# ============================================================================
Log ""
Log "STEP 2: Setting up GitHub CLI (portable)..."

$ghDir = "$env:USERPROFILE\.gh-portable"
$ghExe = "$ghDir\bin\gh.exe"

# Check if gh is already available anywhere
$ghSystem = Get-Command gh -ErrorAction SilentlyContinue
if ($ghSystem) {
    $ghExe = $ghSystem.Source
    Log "  gh already available at: $ghExe"
} elseif (Test-Path $ghExe) {
    Log "  Using portable gh at: $ghExe"
    $env:PATH = "$ghDir\bin;$env:PATH"
} else {
    Log "  Downloading gh CLI portable (no admin required)..."

    # Get latest release info from GitHub API
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/cli/cli/releases/latest" -UseBasicParsing
        $asset = $release.assets | Where-Object { $_.name -like "*windows_amd64.zip" } | Select-Object -First 1
        if (-not $asset) {
            $asset = $release.assets | Where-Object { $_.name -like "*windows_386.zip" } | Select-Object -First 1
        }
        $downloadUrl = $asset.browser_download_url
        $version = $release.tag_name
        Log "  Latest gh version: $version"
        Log "  Download URL: $downloadUrl"
    } catch {
        Log "  Could not fetch release info: $_"
        Fail "Could not get gh CLI download URL."
    }

    # Download the zip
    $zipPath = "$env:TEMP\gh_portable.zip"
    Log "  Downloading to $zipPath ..."
    try {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
        Log "  Download complete."
    } catch {
        Fail "Download failed: $_"
    }

    # Extract
    $extractDir = "$env:TEMP\gh_extract"
    if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
    Log "  Extracting..."
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

    # Find the gh.exe in the extracted folder
    $foundExe = Get-ChildItem -Path $extractDir -Name "gh.exe" -Recurse | Select-Object -First 1
    if (-not $foundExe) { Fail "gh.exe not found in extracted zip." }

    $foundExeDir = Split-Path (Get-ChildItem -Path $extractDir -Filter "gh.exe" -Recurse | Select-Object -First 1 -ExpandProperty FullName)

    # Move to permanent location
    if (-not (Test-Path $ghDir)) { New-Item -ItemType Directory -Path "$ghDir\bin" -Force | Out-Null }
    Copy-Item "$foundExeDir\gh.exe" "$ghExe" -Force
    $env:PATH = "$ghDir\bin;$env:PATH"

    # Cleanup
    Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
    Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue

    Log "  gh CLI ready at: $ghExe"
}

$ghVer = & $ghExe --version 2>&1 | Select-Object -First 1
Log "  Version: $ghVer"

# ============================================================================
# STEP 3: GitHub Authentication (device flow - opens browser automatically)
# ============================================================================
Log ""
Log "STEP 3: GitHub authentication..."
Log "  Checking existing auth..."

$authOut = & $ghExe auth status 2>&1 | Out-String
$authCode = $LASTEXITCODE

if ($authCode -ne 0) {
    Log "  Not authenticated."
    Log "  Starting GitHub device flow - browser will open automatically."
    Log "  If prompted, click Authorize GitHub CLI."
    Log ""
    # This opens the browser and waits for the user to authorize
    & $ghExe auth login --git-protocol https --web
    $code = $LASTEXITCODE
    if ($code -ne 0) { Fail "GitHub auth failed (exit $code)." }
    Log "  GitHub auth complete."
} else {
    Log "  Already authenticated."
}

$ghUser = & $ghExe api user --jq .login 2>&1
Log "  GitHub user: $ghUser"

# ============================================================================
# STEP 4: Create GitHub repo and push
# ============================================================================
Log ""
Log "STEP 4: Creating GitHub repo and pushing..."

$repoName = "rustrelay"
$repoFullName = "$ghUser/$repoName"

& $ghExe repo view $repoFullName 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) {
    Log "  Creating repo $repoFullName..."
    $out = & $ghExe repo create $repoName --public --description "RustRelay: high-performance real-time messaging backend in Rust" 2>&1
    $code = $LASTEXITCODE
    Log "  $out"
    if ($code -ne 0) { Fail "Could not create repo (exit $code)." }
} else {
    Log "  Repo already exists."
}

# Set remote and push
git remote remove origin 2>&1 | Out-Null
$remoteUrl = "https://github.com/$repoFullName.git"
git remote add origin $remoteUrl 2>&1 | Out-Null

Log "  Pushing to $remoteUrl ..."
$out = git push -u origin main 2>&1
$code = $LASTEXITCODE
foreach ($line in $out) { Log "  $line" }
if ($code -ne 0) {
    $out = git push -u origin master 2>&1
    $code = $LASTEXITCODE
    foreach ($line in $out) { Log "  $line" }
}
if ($code -ne 0) { Fail "git push failed (exit $code)." }
Log "  Code pushed to GitHub: https://github.com/$repoFullName"

# ============================================================================
# STEP 5: Open Render deploy URL in browser
# ============================================================================
Log ""
Log "STEP 5: Opening Render for deployment..."

$renderDeployUrl = "https://dashboard.render.com/select-repo?type=web"
$renderBlueprintUrl = "https://render.com/deploy?repo=https://github.com/$repoFullName"

Log "  Opening Render dashboard in browser..."
Log "  Sign in with GitHub (one click) and connect your rustrelay repo."
Log "  The render.yaml in the repo auto-configures everything."

Start-Process $renderBlueprintUrl

Log ""
Log "  Render deploy URL: $renderBlueprintUrl"
Log "  After deploying, Render will give you a URL like:"
Log "    https://rustrelay.onrender.com"
Log ""

$renderUrl = Read-Host "  Paste your Render service URL here (e.g. https://rustrelay.onrender.com)"
if ([string]::IsNullOrWhiteSpace($renderUrl)) {
    Log "  No URL provided. Using placeholder."
    $renderUrl = "https://rustrelay.onrender.com"
}
$renderUrl = $renderUrl.Trim().TrimEnd("/")

# ============================================================================
# STEP 6: Update frontend
# ============================================================================
Log ""
Log "STEP 6: Updating Vercel frontend..."

$wsUrl = $renderUrl -replace "^https://", "wss://"
if (-not $wsUrl.EndsWith("/ws")) { $wsUrl = "$wsUrl/ws" }
Log "  WebSocket URL: $wsUrl"

$htmlPath = "D:\rustrelay\web\index.html"
$content = Get-Content $htmlPath -Raw -Encoding utf8
$content = $content -replace 'value="ws[s]?://[^"]*"', "value=`"$wsUrl`""
Set-Content $htmlPath $content -NoNewline -Encoding utf8
Log "  index.html updated."

# ============================================================================
# STEP 7: Redeploy Vercel
# ============================================================================
Log ""
Log "STEP 7: Redeploying Vercel frontend..."
Set-Location "D:\rustrelay\web"

$vercelCmd = Get-Command vercel -ErrorAction SilentlyContinue
if ($vercelCmd) {
    vercel --prod --yes 2>&1 | ForEach-Object { Log "  $_" }
    Log "  Vercel redeployed."
} else {
    Log "  vercel not found. Run run_redeploy.vbs to redeploy."
}

# ============================================================================
# DONE
# ============================================================================
Log ""
Log "========================================"
Log "DONE"
Log "========================================"
Log "  GitHub:   https://github.com/$repoFullName"
Log "  Render:   $renderUrl"
Log "  WS URL:   $wsUrl"
Log "  Frontend: https://web-sigma-three-51.vercel.app"
Log ""
Log "Full log: $logFile"

Read-Host "Press Enter to close"
