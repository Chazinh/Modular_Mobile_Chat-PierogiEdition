@echo off
chcp 65001 >nul
title ModularChat server
cd /d "%~dp0"
echo ModularChat server
echo.

set "ARGS=%*"

rem Atalhos rapidos passados pela linha de comando
if "%~1"=="2" set "ARGS=-With9Router"
if "%~1"=="1" set "ARGS="
if /i "%~1"=="9router" set "ARGS=-With9Router"
if /i "%~1"=="-9router" set "ARGS=-With9Router"
if /i "%~1"=="--9router" set "ARGS=-With9Router"
if /i "%~1"=="/9router" set "ARGS=-With9Router"
if /i "%~1"=="-with9router" set "ARGS=-With9Router"

rem Menu interativo apenas quando executado sem nenhum argumento (ex: duplo clique)
if "%~1"=="" (
    echo Opcoes de inicializacao:
    echo   [1] Iniciar somente ModularChat
    echo   [2] Iniciar ModularChat + 9Router
    echo.
    choice /c 12 /t 5 /d 1 /m "Selecione uma opcao (padrao [1] em 5s): "
    if errorlevel 2 (
        set "ARGS=-With9Router"
    )
    echo.
)

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-server.ps1" %ARGS%
set "EXITCODE=%errorlevel%"
echo.
if %EXITCODE% neq 0 (
    echo The server stopped with code %EXITCODE%. See the log above; your data was preserved.
) else (
    echo The server was stopped.
)
echo.
pause
