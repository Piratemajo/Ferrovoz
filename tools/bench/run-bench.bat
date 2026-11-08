@echo off
setlocal
chcp 65001 >nul
rem Benchmark de Ferrovoz: tres pasadas seguidas en un servidor de Fabric de pruebas.
rem Pon este archivo y bench_clients.exe en la carpeta del servidor y ejecútalo.
rem Cada pasada carga chunks y apaga el servidor al terminar: no lo uses en el de verdad.

set SERVER_JAR=server.jar
set MEMORY=4G
set SECONDS=120

if not exist "%SERVER_JAR%" (
    echo No encuentro %SERVER_JAR%. Cambia SERVER_JAR al principio de este archivo.
    pause
    exit /b 1
)
if not exist bench_clients.exe (
    echo Falta bench_clients.exe junto a este archivo ^(viene en bench-clients-windows-x86_64 del CI^).
    pause
    exit /b 1
)
findstr /c:"pause-when-empty-seconds=0" server.properties >nul 2>&1 || (
    echo Pon pause-when-empty-seconds=0 en server.properties: si no, el servidor deja de hacer ticks sin jugadores.
    pause
    exit /b 1
)
if not exist ferrovoz-bench-results mkdir ferrovoz-bench-results

rem             nombre        jugadores  colocación  %% hablando  hilos que reciben
call :run       normal        100        spread      20           ""
call :run       evento        250        crowd       60           ""
call :run       evento-1hilo  250        crowd       60           1

echo.
echo Listo. Los informes están en ferrovoz-bench-results: mándalos.
pause
exit /b 0

:run
echo.
echo === %1: %2 jugadores, %4 %% hablando ===
if exist ferrovoz-bench rmdir /s /q ferrovoz-bench
set FERROVOZ_RELAY_THREADS=%~5
start "ferrovoz-clientes" /min bench_clients.exe ferrovoz-bench
java -Xms%MEMORY% -Xmx%MEMORY% -Dferrovoz.bench=%2 -Dferrovoz.bench.layout=%3 -Dferrovoz.bench.talking=%4 -Dferrovoz.bench.seconds=%SECONDS% -jar "%SERVER_JAR%" nogui
rem Los clientes guardan lo que han recibido justo después de que pare el servidor.
timeout /t 5 /nobreak >nul
if not exist ferrovoz-bench-results\%1 mkdir ferrovoz-bench-results\%1
for %%f in (report.md result.properties clients.properties) do (
    if exist ferrovoz-bench\%%f copy /y ferrovoz-bench\%%f ferrovoz-bench-results\%1\ >nul
)
exit /b 0
