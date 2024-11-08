@echo off

rem Run the application

cd /d "%~dp0"

cargo run --package newslookout --release --bin newslookout_app conf\newslookout.toml

if errorlevel=1 pause

rem -- END of FILE --
