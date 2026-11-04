# Compila el jar de Ferrovoz en Windows con las plataformas que puedas:
#   - Windows x64: aquí mismo.
#   - Linux x64: dentro de WSL (tools/build-linux.sh), si tienes WSL.
#   - macOS: no se puede compilar desde Windows. Descarga "natives-macos" del workflow
#     (Actions -> build -> Run workflow -> plataformas: macos) y descomprímelo en
#     mod\natives-prebuilt\ antes de ejecutar esto; se incluirá solo.
#
# Uso (desde la carpeta del proyecto):   powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1
# Sin Linux:                              ... -File tools\build-windows.ps1 -SinLinux
param([switch]$SinLinux, [switch]$SinTests)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location "$root\native"

if (-not $SinTests) {
    Write-Host "== Tests del núcleo"
    cargo test --release
    if ($LASTEXITCODE -ne 0) { throw "Fallan los tests" }
}

Write-Host "== Windows x64: cliente (con audio)"
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Falló la compilación (cliente)" }
Write-Host "== Windows x64: servidor (sin audio)"
cargo build --release --no-default-features --target-dir target\server
if ($LASTEXITCODE -ne 0) { throw "Falló la compilación (servidor)" }
$out = "$root\mod\natives-prebuilt\natives\windows-x86_64"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "target\release\ferrovoz.dll" "$out\ferrovoz.dll" -Force
Copy-Item "target\server\release\ferrovoz.dll" "$out\ferrovoz_server.dll" -Force

if (-not $SinLinux) {
    if (Get-Command wsl -ErrorAction SilentlyContinue) {
        Write-Host "== Linux x64 (WSL)"
        Set-Location $root
        wsl bash tools/build-linux.sh
        if ($LASTEXITCODE -ne 0) { throw "Falló Linux en WSL (mira el mensaje de arriba)" }
    } else {
        Write-Warning "No tienes WSL: el jar no llevará Linux. Instálalo con: wsl --install"
    }
}

Write-Host "== Jar"
Set-Location "$root\mod"
.\gradlew.bat build -PskipNative
if ($LASTEXITCODE -ne 0) { throw "Falló Gradle" }

$natives = "$root\mod\natives-prebuilt\natives"
$platforms = Get-ChildItem $natives -Directory | ForEach-Object { $_.Name }
Write-Host ""
Write-Host "Plataformas en el jar: $($platforms -join ', ')"
foreach ($p in @('windows-x86_64', 'linux-x86_64', 'macos-aarch64', 'macos-x86_64')) {
    if ($platforms -notcontains $p) { Write-Warning "Falta $p" }
}
Get-ChildItem "$root\mod\build\libs\*.jar" | Where-Object { $_.Name -notlike '*-sources*' } | ForEach-Object { Write-Host "Jar: $($_.FullName)" }
