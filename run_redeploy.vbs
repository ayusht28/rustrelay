Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\rustrelay\redeploy_frontend.ps1", 1, False
