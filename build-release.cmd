@echo off
setlocal

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build-release.ps1"
set EXIT_CODE=%ERRORLEVEL%

if not "%EXIT_CODE%"=="0" (
  echo.
  echo Build failed with exit code %EXIT_CODE%.
  pause
  exit /b %EXIT_CODE%
)

echo.
echo Done. Press any key to close.
pause >nul
