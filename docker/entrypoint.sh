#!/bin/sh
# Container entrypoint: optionally copy a Rekordbox library snapshot to local
# disk, then run reklawdbox (HTTP mode when REKLAWDBOX_HTTP_ADDR is set).
#
# REKLAWDBOX_SNAPSHOT_FROM  directory holding master.db (+ -wal/-shm, share/),
#                           usually a read-only network mount. It is copied to
#                           the directory of REKORDBOX_DB_PATH so SQLite never
#                           opens the database over NFS/SMB, where its locking
#                           is unreliable. Restart the container to pick up a
#                           newer snapshot.
# REKLAWDBOX_SNAPSHOT_ANLZ  1 = also copy share/ (Rekordbox beat grids).
set -eu

if [ -n "${REKLAWDBOX_SNAPSHOT_FROM:-}" ]; then
    src="$REKLAWDBOX_SNAPSHOT_FROM"
    dest="$(dirname "${REKORDBOX_DB_PATH:?REKORDBOX_DB_PATH must be set}")"
    [ -f "$src/master.db" ] || { echo "entrypoint: no master.db in $src" >&2; exit 1; }
    mkdir -p "$dest"
    staging="$(mktemp -d "$dest/.copy-XXXXXX")"
    for name in master.db master.db-wal master.db-shm SNAPSHOT; do
        if [ -f "$src/$name" ]; then
            cp "$src/$name" "$staging/$name"
            # A copy cut short by the network must not become the library.
            [ "$(wc -c < "$src/$name")" -eq "$(wc -c < "$staging/$name")" ] \
                || { echo "entrypoint: $name copied short" >&2; exit 1; }
        fi
    done
    if [ "${REKLAWDBOX_SNAPSHOT_ANLZ:-0}" = 1 ] && [ -d "$src/share" ]; then
        cp -r "$src/share" "$staging/share"
    fi
    rm -rf "$dest/master.db" "$dest/master.db-wal" "$dest/master.db-shm" "$dest/SNAPSHOT" "$dest/share"
    mv "$staging"/* "$dest"/
    rmdir "$staging"
    echo "entrypoint: snapshot from $src copied to $dest ($(wc -c < "$dest/master.db") bytes)" >&2
    [ -f "$dest/SNAPSHOT" ] && sed 's/^/entrypoint:   /' "$dest/SNAPSHOT" >&2
fi

exec /usr/local/bin/reklawdbox "$@"
