param([string]$Destination = "target/body-webview-driver")
$ErrorActionPreference = "Stop"

# Match the WebView2 Runtime, not the unrelated Microsoft Edge browser version.
$runtimeVersion = $null
if ($env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER) {
    $runtime = Join-Path $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER "msedgewebview2.exe"
    $runtimeVersion = (Get-Item $runtime).VersionInfo.ProductVersion
} else {
    $client = "Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
    $keys = @(
        "HKLM:\SOFTWARE\WOW6432Node\$client", "HKLM:\SOFTWARE\$client",
        "HKCU:\SOFTWARE\WOW6432Node\$client", "HKCU:\SOFTWARE\$client"
    )
    foreach ($key in $keys) {
        $record = Get-ItemProperty -Path $key -ErrorAction SilentlyContinue
        if ($record -and $record.pv -match '^\d+\.\d+\.\d+\.\d+$') {
            $runtimeVersion = $record.pv
            break
        }
    }
}
if ($runtimeVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "A versioned WebView2 Runtime is required; no browser-version fallback is used."
}
$platform = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
    "X64" { "win64" }
    "Arm64" { "arm64" }
    "X86" { "win32" }
    default { throw "Unsupported Windows driver architecture" }
}
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
[System.IO.Directory]::CreateDirectory($destinationPath) | Out-Null
$archivePath = Join-Path $destinationPath "edgedriver.zip"
$driverPath = Join-Path $destinationPath "msedgedriver.exe"
$download = "https://msedgedriver.microsoft.com/$runtimeVersion/edgedriver_$platform.zip"
Invoke-WebRequest -Uri $download -OutFile $archivePath
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $entry = $archive.GetEntry("msedgedriver.exe")
    if (-not $entry) { throw "Microsoft driver archive lacks msedgedriver.exe" }
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $driverPath, $true)
} finally {
    $archive.Dispose()
}
$driverVersion = (& $driverPath --version) -join " "
if ($LASTEXITCODE -ne 0 -or $driverVersion -notmatch [regex]::Escape($runtimeVersion)) {
    throw "Downloaded driver version does not match WebView2 Runtime $runtimeVersion"
}
@{
    runtimeVersion = $runtimeVersion
    driverVersion = $driverVersion
    driverPath = $driverPath
    download = $download
} | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $destinationPath "versions.json")
Write-Output $driverPath
