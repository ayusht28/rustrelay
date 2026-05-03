Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\rustrelay\update_frontend_url.ps1", 1, False
