param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$BuildDirectory = 'src-tauri/target/release',
  [string]$ToolsDirectory = 'src-tauri/tools',
  [string]$ExtensionDirectory = 'browser-extension',
  [string]$OutputDirectory = 'release'
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release version.' }
$buildRoot = (Resolve-Path -LiteralPath $BuildDirectory).Path
$toolsRoot = (Resolve-Path -LiteralPath $ToolsDirectory).Path
$extensionRoot = (Resolve-Path -LiteralPath $ExtensionDirectory).Path
$extensionManifest = Get-Content -LiteralPath (Join-Path $extensionRoot 'manifest.json') -Raw | ConvertFrom-Json
if ($extensionManifest.version -ne $Version) { throw 'Extension version does not match the release.' }
foreach ($requiredTool in @('node.exe', 'yt-dlp.exe', 'node-LICENSE.txt', 'yt-dlp-LICENSE.txt')) {
  if (-not (Test-Path -LiteralPath (Join-Path $toolsRoot $requiredTool) -PathType Leaf)) { throw "Missing media resource: $requiredTool" }
}
$binary = Join-Path $buildRoot 'ARGON.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'ARGON.exe was not built.' }
$installers = @(Get-ChildItem -LiteralPath (Join-Path $buildRoot 'bundle/nsis') -Filter '*-setup.exe' -File)
if ($installers.Count -ne 1) { throw 'Expected exactly one NSIS installer in this build.' }
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stagingRoot = Join-Path $buildRoot "argon-portable-$Version"
if (Test-Path -LiteralPath $stagingRoot) { throw 'Portable staging directory already exists; use a clean build directory.' }
New-Item -ItemType Directory -Path $stagingRoot | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stagingRoot 'ARGON.exe')
Copy-Item -LiteralPath $toolsRoot -Destination (Join-Path $stagingRoot 'tools') -Recurse
Copy-Item -LiteralPath $extensionRoot -Destination (Join-Path $stagingRoot 'browser-extension') -Recurse
@"
ARGON $Version
Extract the complete ZIP into a writable folder before starting ARGON.exe.
Keep tools/ and browser-extension/ beside ARGON.exe. Browser setup is inside the app.
"@ | Set-Content -LiteralPath (Join-Path $stagingRoot 'START-HERE.txt') -Encoding utf8
$portableZip = Join-Path $outputRoot "argon-$Version-portable.zip"
Compress-Archive -Path (Join-Path $stagingRoot '*') -DestinationPath $portableZip
Compress-Archive -LiteralPath $extensionRoot -DestinationPath (Join-Path $outputRoot "argon-$Version-browser-extension.zip")
Copy-Item -LiteralPath $installers[0].FullName -Destination (Join-Path $outputRoot "ARGON_${Version}_x64-setup.exe")
if (Test-Path -LiteralPath "$($installers[0].FullName).sig") {
  Copy-Item -LiteralPath "$($installers[0].FullName).sig" -Destination (Join-Path $outputRoot "ARGON_${Version}_x64-setup.exe.sig")
}
else { throw 'A signed installer is required for in-app updates.' }
$archive = [System.IO.Compression.ZipFile]::OpenRead($portableZip)
try {
  $entries = @($archive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
  foreach ($requiredEntry in @('ARGON.exe', 'tools/node.exe', 'tools/yt-dlp.exe', 'browser-extension/manifest.json', 'browser-extension/background.js', 'browser-extension/bridge-client.js', 'browser-extension/core.js', 'browser-extension/popup.js', 'browser-extension/popup.css', 'browser-extension/video-button.js')) {
    if ($entries -notcontains $requiredEntry) { throw "Portable archive is missing $requiredEntry" }
  }
} finally { $archive.Dispose() }
Write-Output "Validated installer, portable ZIP with media resources, and browser extension for ARGON $Version."
