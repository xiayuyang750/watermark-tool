@echo off
rem One-click launcher (double-click friendly, bypasses PS execution policy)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1" %*
