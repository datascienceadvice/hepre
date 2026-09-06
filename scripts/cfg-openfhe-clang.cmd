@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
where clang-cl
cmake -S D:\lair\hepre\vendor\openfhe-development -B D:\lair\hepre\vendor\openfhe-clang-build -G "NMake Makefiles" -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DCMAKE_BUILD_TYPE=Release -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL -DCMAKE_SYSTEM_VERSION=10.0.26100.0 -DBUILD_STATIC=ON -DBUILD_SHARED=OFF -DBUILD_UNITTESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_BENCHMARKS=OFF -DWITH_OPENMP=OFF -DWITH_NTL=OFF -DBUILD_EXTRAS=OFF -DCMAKE_INSTALL_PREFIX=D:\lair\hepre\vendor\openfhe-install
echo CONFIG_EXIT=%ERRORLEVEL%