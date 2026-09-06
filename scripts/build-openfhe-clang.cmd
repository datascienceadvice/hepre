@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cmake --build D:\lair\hepre\vendor\openfhe-clang-build --config Release 2>&1
echo BUILD_EXIT=%ERRORLEVEL%