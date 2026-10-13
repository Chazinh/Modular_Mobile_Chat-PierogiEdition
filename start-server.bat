@echo off
chcp 65001 >nul
title ModularChat server
cd /d "%~dp0"
echo ModularChat server
echo.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-server.ps1" %*
set "EXITCODE=%errorlevel%"
echo.
if %EXITCODE% neq 0 (
    echo The server stopped with code %EXITCODE%. See the log above; your data was preserved.
) else (
    echo The server was stopped.
)
echo.
pause
