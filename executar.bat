@echo off
rem Dois cliques: compila o BigPicture com o codigo atual e abre o launcher (veja executar.ps1).
rem Argumentos vao para o launcher, por exemplo: executar.bat --monitor 1
rem Usa o PowerShell 7 se estiver instalado, senao o do Windows.
set "PS=powershell"
where pwsh >nul 2>nul && set "PS=pwsh"
%PS% -NoProfile -ExecutionPolicy Bypass -File "%~dp0executar.ps1" %*
