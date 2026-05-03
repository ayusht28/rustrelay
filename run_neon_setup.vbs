Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\rustrelay\setup_neon_db.ps1", 1, False
