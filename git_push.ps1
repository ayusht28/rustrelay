Set-Location "D:\rustrelay\rustrelay"
$logFile = "D:\rustrelay\git_push.log"
"=== Git Push Log ===" | Out-File $logFile -Encoding utf8

function Log($msg) {
    $msg | Out-File $logFile -Append -Encoding utf8
    Write-Host $msg
}

Log "Initialising git repo..."
git init 2>&1 | ForEach-Object { Log $_ }

Log "Configuring git identity..."
git config user.email "ayushthakare122005@gmail.com" 2>&1 | Out-Null
git config user.name "Ayush Thakare" 2>&1 | Out-Null

Log "Adding all files..."
git add . 2>&1 | ForEach-Object { Log $_ }

Log "Committing..."
git commit -m "Initial commit: RustRelay backend" 2>&1 | ForEach-Object { Log $_ }

Log "Adding remote origin..."
git remote remove origin 2>&1 | Out-Null
git remote add origin "https://github.com/ayushthakare122005/rustrelay.git" 2>&1 | ForEach-Object { Log $_ }

Log "Pushing to GitHub (a browser or credential prompt may appear)..."
git push -u origin main 2>&1 | ForEach-Object { Log $_ }
if ($LASTEXITCODE -ne 0) {
    Log "Trying master branch..."
    git push -u origin master 2>&1 | ForEach-Object { Log $_ }
}

if ($LASTEXITCODE -eq 0) {
    Log ""
    Log "SUCCESS: Code pushed to https://github.com/ayushthakare122005/rustrelay"
} else {
    Log ""
    Log "PUSH FAILED (exit $LASTEXITCODE) - check log"
}

Read-Host "Press Enter to close"
