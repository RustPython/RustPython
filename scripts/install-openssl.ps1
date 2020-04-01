# From the Actix Web windows workflow:
# https://github.com/actix/actix-web/blob/master/.github/workflows/windows.yml
vcpkg integrate install
vcpkg install openssl:x64-windows
Copy-Item C:\vcpkg\installed\x64-windows\bin\libcrypto-1_1-x64.dll C:\vcpkg\installed\x64-windows\bin\libcrypto.dll
Copy-Item C:\vcpkg\installed\x64-windows\bin\libssl-1_1-x64.dll C:\vcpkg\installed\x64-windows\bin\libssl.dll
Get-ChildItem C:\vcpkg\installed\x64-windows\bin
Get-ChildItem C:\vcpkg\installed\x64-windows\lib
