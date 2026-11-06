<#
.SYNOPSIS
    Publish a snapshot of the Windows Rekordbox library for a reklawdbox
    container (docs/linux.md, "Container").

.DESCRIPTION
    Copies master.db (and -wal/-shm when present) from %APPDATA%\Pioneer\rekordbox
    to -Destination, usually a network share the container mounts read-only.
    The container copies the snapshot to local disk on start, so it never opens
    the database over the network.

    Refuses while rekordbox.exe is running (the copy could be torn); -Force
    overrides. The new snapshot replaces the old one only after every file was
    copied in full.

.EXAMPLE
    scripts\publish-snapshot.ps1 -Destination N:\rekordbox\snapshot
.EXAMPLE
    scripts\publish-snapshot.ps1 -Destination N:\rekordbox\snapshot -WithAnlz
#>
param(
    [Parameter(Mandatory = $true)][string]$Destination,
    [string]$Source = (Join-Path $env:APPDATA 'Pioneer\rekordbox'),
    [switch]$WithAnlz,
    [switch]$Force
)
$ErrorActionPreference = 'Stop'

$db = Join-Path $Source 'master.db'
if (-not (Test-Path -LiteralPath $db -PathType Leaf)) { throw "no master.db in $Source (use -Source)" }

if (Get-Process -Name rekordbox -ErrorAction SilentlyContinue) {
    if (-not $Force) { throw 'Rekordbox is running. Close it so the snapshot is consistent, or pass -Force.' }
    Write-Warning 'Rekordbox is running; the copy may be inconsistent.'
}

$parent = Split-Path -Parent $Destination
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$staging = "$Destination.new"
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging | Out-Null

foreach ($name in 'master.db', 'master.db-wal', 'master.db-shm') {
    $from = Join-Path $Source $name
    if (-not (Test-Path -LiteralPath $from -PathType Leaf)) { continue }
    $to = Join-Path $staging $name
    Copy-Item -LiteralPath $from -Destination $to
    if ((Get-Item -LiteralPath $from).Length -ne (Get-Item -LiteralPath $to).Length) { throw "$name copied short" }
}
if ($WithAnlz) {
    $share = Join-Path $Source 'share'
    if (-not (Test-Path -LiteralPath $share -PathType Container)) { throw "no share\ directory in $Source" }
    Copy-Item -LiteralPath $share -Destination (Join-Path $staging 'share') -Recurse
}
$stamp = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
"source=$Source`ncopied_at=$stamp`nhost=$env:COMPUTERNAME`n" |
    Set-Content -LiteralPath (Join-Path $staging 'SNAPSHOT') -NoNewline -Encoding ascii

if (Test-Path -LiteralPath $Destination) {
    $old = "$Destination.old"
    if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Recurse -Force }
    Rename-Item -LiteralPath $Destination -NewName (Split-Path -Leaf $old)
    Rename-Item -LiteralPath $staging -NewName (Split-Path -Leaf $Destination)
    Remove-Item -LiteralPath $old -Recurse -Force
} else {
    Rename-Item -LiteralPath $staging -NewName (Split-Path -Leaf $Destination)
}

$size = '{0:N1} MB' -f ((Get-Item -LiteralPath (Join-Path $Destination 'master.db')).Length / 1MB)
Write-Output "Snapshot ready: $Destination (master.db $size, $stamp)"
Write-Output 'Restart the reklawdbox container to load it.'
