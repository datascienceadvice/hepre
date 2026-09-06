@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set OPENFHE=D:\lair\hepre\vendor\openfhe-install
set LLVM=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64
clang-cl /nologo /MD /EHsc /O2 /Zi /std:c++17 /D_USE_MATH_DEFINES /I"%OPENFHE%\include\openfhe\pke" /I"%OPENFHE%\include\openfhe\core" /I"%OPENFHE%\include\openfhe\binfhe" /I"%OPENFHE%\include\openfhe" %1 /Fe:%2 /link /LIBPATH:"%OPENFHE%\lib" /LIBPATH:"%LLVM%\lib\clang\19\lib\windows" OPENFHEpke_static.lib OPENFHEcore_static.lib OPENFHEbinfhe_static.lib clang_rt.builtins-x86_64.lib /DEBUG 2>&1
echo EXITCODE=%ERRORLEVEL%
