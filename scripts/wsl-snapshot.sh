#!/bin/bash
# reklawdbox: copy the Windows Rekordbox library into WSL as a read-only snapshot.
#
# reklawdbox on Linux never opens the live Windows master.db: SQLite's WAL
# locking does not work across the Windows/WSL file-system boundary, so a
# reader in WSL can see a torn database while Rekordbox writes. Instead it
# reads a copy taken while Rekordbox is closed.
#
# Usage:
#   scripts/wsl-snapshot.sh                      # copy master.db (+ -wal/-shm)
#   scripts/wsl-snapshot.sh --with-anlz          # also copy share/ (beat grids)
#   scripts/wsl-snapshot.sh --dest DIR           # default: ~/.local/share/reklawdbox/rekordbox
#   scripts/wsl-snapshot.sh --source DIR         # default: %APPDATA%\Pioneer\rekordbox
#   scripts/wsl-snapshot.sh --force              # copy even if Rekordbox is running
#
# Afterwards point reklawdbox at the copy:
#   REKORDBOX_DB_PATH=<dest>/master.db

set -euo pipefail

DEST="${XDG_DATA_HOME:-$HOME/.local/share}/reklawdbox/rekordbox"
SOURCE=""
WITH_ANLZ=0
FORCE=0

die() { echo "error: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dest) DEST="${2:?--dest needs a directory}"; shift 2 ;;
        --source) SOURCE="${2:?--source needs a directory}"; shift 2 ;;
        --with-anlz) WITH_ANLZ=1; shift ;;
        --force) FORCE=1; shift ;;
        -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null \
    || die "this script copies a Windows library and must run inside WSL"

# WSL interop (running .exe files) is often off — e.g. systemd=true can drop
# its binfmt registration — so every Windows call has a fallback.
interop=0
cmd.exe /c exit 0 < /dev/null > /dev/null 2>&1 && interop=1

if [[ -z "$SOURCE" ]]; then
    if [[ $interop -eq 1 ]]; then
        appdata="$(cmd.exe /c 'echo %APPDATA%' < /dev/null 2> /dev/null | tr -d '\r')"
        [[ -n "$appdata" && "$appdata" != "%APPDATA%" ]] && SOURCE="$(wslpath -u "$appdata")/Pioneer/rekordbox"
    fi
    if [[ -z "$SOURCE" ]]; then
        shopt -s nullglob
        candidates=(/mnt/c/Users/*/AppData/Roaming/Pioneer/rekordbox/master.db)
        shopt -u nullglob
        [[ ${#candidates[@]} -eq 1 ]] \
            || die "found ${#candidates[@]} Rekordbox libraries under /mnt/c/Users; pick one with --source"
        SOURCE="$(dirname "${candidates[0]}")"
    fi
fi
[[ -f "$SOURCE/master.db" ]] || die "no master.db in $SOURCE (use --source)"

if [[ $interop -eq 1 ]]; then
    if tasklist.exe /FI "IMAGENAME eq rekordbox.exe" /NH < /dev/null 2> /dev/null | grep -qi "rekordbox.exe"; then
        busy="Rekordbox is running"
    fi
elif [[ -e "$SOURCE/master.db-shm" ]]; then
    # Without interop the Windows process list is out of reach; SQLite keeps
    # master.db-shm while a connection has the database open.
    busy="cannot see Windows processes (WSL interop is off) and master.db-shm exists, so the library looks open"
fi
if [[ -n "${busy:-}" ]]; then
    if [[ $FORCE -eq 1 ]]; then
        echo "warning: $busy; the copy may be inconsistent." >&2
    else
        die "$busy. Close Rekordbox so the snapshot is consistent, or pass --force."
    fi
fi

parent="$(dirname "$DEST")"
mkdir -p "$parent"
staging="$(mktemp -d "$parent/.snapshot-XXXXXX")"
trap 'rm -rf "$staging"' EXIT
chmod 700 "$staging"

for name in master.db master.db-wal master.db-shm; do
    [[ -f "$SOURCE/$name" ]] && cp --preserve=timestamps "$SOURCE/$name" "$staging/$name"
done
if [[ $WITH_ANLZ -eq 1 ]]; then
    [[ -d "$SOURCE/share" ]] || die "no share/ directory in $SOURCE"
    cp -r --preserve=timestamps "$SOURCE/share" "$staging/share"
fi
find "$staging" -type d -exec chmod 700 {} + && find "$staging" -type f -exec chmod 600 {} +
printf 'source=%s\ncopied_at=%s\n' "$SOURCE" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$staging/SNAPSHOT"

# Swap in the new snapshot; keep the previous one until the swap succeeded.
if [[ -e "$DEST" ]]; then
    old="$DEST.old.$$"
    mv "$DEST" "$old"
    mv "$staging" "$DEST"
    rm -rf "$old"
else
    mv "$staging" "$DEST"
fi
trap - EXIT

size="$(du -sh "$DEST" | cut -f1)"
echo "Snapshot ready: $DEST ($size)"
echo "  export REKORDBOX_DB_PATH=$DEST/master.db"
