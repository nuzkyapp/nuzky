$ErrorActionPreference = 'Stop'

# BtbN retains month-end builds for two years; daily pins disappear after 14 days.
$tag = 'autobuild-2026-09-30-13-08'
$name = 'ffmpeg-n8.1.3-9-g29e619e767-win64-gpl-shared-8.1'
$sha256 = 'dfe81c3aa0a546ee81980b1b824486dd0fe0a1d452f00cb55135379a74be7bcf'
$deps = Join-Path $env:LOCALAPPDATA 'Nuzky/build-deps'
New-Item -ItemType Directory -Force $deps | Out-Null
$archive = Join-Path $deps "$name.zip"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$tag/$name.zip"
if (!(Test-Path $archive)) {
    Invoke-WebRequest $url -OutFile $archive -TimeoutSec 120
}
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sha256) {
    throw "FFmpeg checksum mismatch: delete $archive and retry."
}
$env:FFMPEG_DIR = Join-Path $deps $name
# scripts/build-bundles.mjs names the build in build-info, where its source comes from.
$env:FFMPEG_BTBN_TAG = $tag
if (!(Test-Path "$env:FFMPEG_DIR/include/libavcodec/avcodec.h")) {
    Expand-Archive $archive -DestinationPath $deps -Force
}
if (!(Test-Path "$env:FFMPEG_DIR/lib/avcodec.lib")) {
    throw 'The FFmpeg archive must contain MSVC import libraries.'
}
$env:LIBCLANG_PATH = 'C:\Program Files\LLVM\bin'
if (!(Test-Path "$env:LIBCLANG_PATH/libclang.dll")) {
    throw 'Install LLVM x64 (winget install LLVM.LLVM), then rerun this script.'
}
if (!(Get-Command cmake -ErrorAction SilentlyContinue)) {
    throw 'Install CMake (winget install Kitware.CMake), then reopen the terminal.'
}
$env:Path = "$env:FFMPEG_DIR\bin;$env:LIBCLANG_PATH;$env:Path"
if ($env:GITHUB_ENV) {
    "FFMPEG_DIR=$env:FFMPEG_DIR" >> $env:GITHUB_ENV
    "FFMPEG_BTBN_TAG=$tag" >> $env:GITHUB_ENV
    "LIBCLANG_PATH=$env:LIBCLANG_PATH" >> $env:GITHUB_ENV
    "$env:FFMPEG_DIR\bin" >> $env:GITHUB_PATH
    $env:LIBCLANG_PATH >> $env:GITHUB_PATH
}
& "$env:FFMPEG_DIR/bin/ffmpeg.exe" -version
if ($LASTEXITCODE -ne 0) { throw 'FFmpeg could not start.' }
# ONNX Runtime, which debug builds load from the same build-deps directory when covers run.
node "$PSScriptRoot/fetch-onnxruntime.mjs"
if ($LASTEXITCODE -ne 0) { throw 'ONNX Runtime could not be fetched.' }
# Microsoft Edge WebDriver drives the app's WebView2 in the UI flows (scripts/repro.py). It must match the installed
# WebView2, which updates itself, so no checksum can be pinned; Microsoft's signature on it is checked instead.
$runtime = 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$webview = (Get-ItemProperty "HKLM:\$runtime", "HKCU:\$runtime" -ErrorAction SilentlyContinue | Select-Object -First 1).pv
if ($webview -and !$env:GITHUB_ENV) {
    $driver = Join-Path $deps "msedgedriver-$webview/msedgedriver.exe"
    if (!(Test-Path $driver)) {
        $zip = Join-Path $deps "edgedriver-$webview.zip"
        # Windows PowerShell's progress bar slows the download down to minutes.
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest "https://msedgedriver.microsoft.com/$webview/edgedriver_win64.zip" -OutFile $zip -TimeoutSec 300
        Expand-Archive $zip -DestinationPath (Split-Path $driver) -Force
        Remove-Item $zip
    }
    $signature = Get-AuthenticodeSignature $driver
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        Remove-Item -Recurse (Split-Path $driver)
        throw 'The Edge WebDriver downloaded is not signed by Microsoft; run this script again.'
    }
}
