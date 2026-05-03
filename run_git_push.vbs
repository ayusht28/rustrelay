Set WshShell = CreateObject("WScript.Shell")
WshShell.Run "powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\rustrelay\git_push.ps1", 1, False
