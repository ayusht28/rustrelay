Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\rustrelay\deploy_portable.ps1", 1, False
