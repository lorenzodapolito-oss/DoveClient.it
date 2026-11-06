@echo off
rem Scarica (o aggiorna) e compila DoveClient.it. Metti questo file in AmbienteRust.
title Costruisci DoveClient.it
set "ROOT=%~dp0"
if "%ROOT:~-1%"=="\" set "ROOT=%ROOT:~0,-1%"
set "CARGO_HOME=%ROOT%\rust\cargo"
set "RUSTUP_HOME=%ROOT%\rust\rustup"
set "PATH=%ROOT%\winlibs\mingw64\bin;%ROOT%\rust\cargo\bin;%ROOT%\git\cmd;%ROOT%\git\bin;%PATH%"
set "PROG=%ROOT%\Progetti"
set "PROJ=%PROG%\DoveClient.it"
if not exist "%PROG%" mkdir "%PROG%"
cd /d "%PROG%"
taskkill /f /im doveclient.exe >nul 2>&1
echo === Scarico o aggiorno DoveClient.it ===
if exist "%PROJ%\.git" (
  git -c "safe.directory=*" -C "%PROJ%" fetch origin
  if errorlevel 1 goto fail
  git -c "safe.directory=*" -C "%PROJ%" reset --hard origin/main
) else (
  git clone https://github.com/lorenzodapolito-oss/DoveClient.it "%PROJ%"
)
if errorlevel 1 goto fail
cd /d "%PROJ%"
echo === Compilo (la prima volta ci vogliono diversi minuti) ===
cargo build --release
if errorlevel 1 goto fail
set "OUT=%PROJ%\dist\DoveClient-portable"
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%"
copy /y "target\release\doveclient.exe" "%OUT%\DoveClient.exe" >nul
if errorlevel 1 goto fail
echo.
echo FATTO: %OUT%\DoveClient.exe
pause
exit /b 0
:fail
echo.
echo ERRORE: compilazione non riuscita. Controlla i messaggi sopra.
pause
exit /b 1
