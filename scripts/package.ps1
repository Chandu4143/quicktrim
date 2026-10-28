# Build the portable Windows bundle and checksum it.
#
#   powershell -ExecutionPolicy Bypass -File scripts/package.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/package.ps1 -Version 0.1.0
#
# The app comes from target\release\quicktrim.exe, the helpers from
# target\release\ (or dist\quicktrim\ as a fallback). Output lands in dist\ as:
#
#   quicktrim-<version>-win-x64.zip
#   quicktrim-<version>-win-x64.zip.sha256
#
# Nothing outside dist\ is written.

param(
    [string]$Version = "",
    [string]$AppExe = "target\release\quicktrim.exe",
    [string]$HelpersDir = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $line = Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $line) { throw "Could not read the version from Cargo.toml. Pass -Version." }
    $Version = $line.Matches[0].Groups[1].Value
}

if (-not (Test-Path $AppExe)) {
    throw "Missing $AppExe. Run: cargo build --release (with the helpers in target\release\)"
}

$helpers = @("ffmpeg.exe", "ffprobe.exe", "libmpv-2.dll")

if (-not $HelpersDir) {
    $HelpersDir = "target\release"
    if (-not (Test-Path (Join-Path $HelpersDir "libmpv-2.dll"))) {
        $HelpersDir = "dist\quicktrim"
    }
}

foreach ($helper in $helpers) {
    $path = Join-Path $HelpersDir $helper
    if (-not (Test-Path $path)) {
        throw "Missing $path. Copy the helpers into $HelpersDir or pass -HelpersDir."
    }
}

$name = "quicktrim-$Version-win-x64"
$stage = Join-Path "dist" $name
$zip = Join-Path "dist" "$name.zip"

if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "docs") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stage "LICENSES") | Out-Null

# The application and its helpers.
Copy-Item $AppExe (Join-Path $stage "quicktrim.exe")
foreach ($helper in $helpers) {
    Copy-Item (Join-Path $HelpersDir $helper) (Join-Path $stage $helper)
}

# Everything the licence requires to travel with the binary.
Copy-Item "LICENSE" $stage
Copy-Item "THIRD_PARTY_NOTICES.md" $stage
Copy-Item "CHANGELOG.md" $stage
Copy-Item "LICENSES\GPL-2.0.txt" (Join-Path $stage "LICENSES")
Copy-Item "docs\third-party-crates.md" (Join-Path $stage "docs")

Set-Content -Path (Join-Path $stage "VERSION.txt") -Value "QuickTrim $Version (win-x64)" -Encoding ASCII

$readme = @"
QuickTrim $Version - Windows x64

Run quicktrim.exe, then:
  - open a video with the button or drag one onto the window
  - Space plays and pauses, arrow keys skip 5 s (Shift for 1 s)
  - I and O set the cut points, Ctrl+S saves a copy
  - the original file is never modified

Everything needed is in this folder. Nothing is installed, no registry keys
are written, and there is no network access or telemetry. Delete the folder
to remove the app.

Licence: GPL-3.0-or-later. See LICENSE, LICENSES/ and
THIRD_PARTY_NOTICES.md. Source: https://github.com/quicktrim/quicktrim
"@
Set-Content -Path (Join-Path $stage "README.txt") -Value $readme -Encoding ASCII

# SHA256SUMS, sorted, in sha256sum-compatible form (LF line endings).
$stageFull = (Resolve-Path $stage).Path
$sums = Get-ChildItem $stage -Recurse -File |
    Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
    Sort-Object FullName |
    ForEach-Object {
        $relative = $_.FullName.Substring($stageFull.Length + 1).Replace("\", "/")
        $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
        "$hash  $relative"
    }
$sumsText = ($sums -join "`n") + "`n"
[System.IO.File]::WriteAllText(
    (Join-Path $stage "SHA256SUMS.txt"),
    $sumsText,
    (New-Object System.Text.UTF8Encoding($false))
)

if (Test-Path $zip) { Remove-Item $zip -Force }
if (Get-Command tar -ErrorAction SilentlyContinue) {
    tar -a -c -f $zip -C "dist" $name
} else {
    Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
}
if (-not (Test-Path $zip)) { throw "Packaging failed: $zip was not created." }

$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
Set-Content -Path "$zip.sha256" -Value "$zipHash  $name.zip" -Encoding ASCII

$total = (Get-ChildItem $stage -Recurse -File | Measure-Object -Property Length -Sum).Sum
$app = (Get-Item (Join-Path $stage "quicktrim.exe")).Length

""
"  {0}" -f $name
"  app         {0,10:N0} bytes" -f $app
"  bundle      {0,10:N0} bytes ({1:N1} MB)" -f $total, ($total / 1MB)
"  zip         {0,10:N0} bytes" -f (Get-Item $zip).Length
"  sha256      {0}" -f $zipHash
""
"  {0}" -f $zip
""
