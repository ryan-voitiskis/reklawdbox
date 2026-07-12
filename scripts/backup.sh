#!/bin/bash
# reklawdbox: Rekordbox library backup tool
# Snapshots the full Rekordbox library state to timestamped archives.
#
# Usage:
#   ./scripts/backup.sh                  # Full backup
#   ./scripts/backup.sh --db-only        # Database files only (fast, ~50MB)
#   ./scripts/backup.sh --list           # List existing backups
#   ./scripts/backup.sh --restore <path> # Restore from a backup archive
#
# Backup location: ~/Music/rekordbox-backups/

set -euo pipefail

# --- Configuration ---
DEFAULT_RB_DATA="$HOME/Library/Pioneer/rekordbox"
RB_DATA="$DEFAULT_RB_DATA"
EFFECTIVE_DB_PATH=""
BACKUP_DIR="$HOME/Music/rekordbox-backups"
MAX_FULL_BACKUPS=5
MAX_DB_BACKUPS=20
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# --- Colors ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()  { echo -e "${GREEN}[backup]${NC} $*"; }
warn() { echo -e "${YELLOW}[backup]${NC} $*"; }
err()  { echo -e "${RED}[backup]${NC} $*" >&2; }

# --- Preflight checks ---
canonical_parent_directory() {
    local path="$1"
    local parent
    parent="$(dirname "$path")"
    if [[ ! -d "$parent" ]]; then
        err "Rekordbox data directory not found: $parent"
        exit 1
    fi
    (cd "$parent" && pwd -P)
}

configure_backup_source() {
    local configured="${REKORDBOX_DB_PATH:-}"
    local source_path
    if [[ -n "$configured" ]]; then
        source_path="$configured"
    else
        source_path="$DEFAULT_RB_DATA/master.db"
    fi

    if [[ -L "$source_path" ]]; then
        err "Rekordbox database path must be a direct regular file; symlinks are not supported: $source_path"
        exit 1
    fi
    if [[ "$(basename "$source_path")" != "master.db" ]]; then
        err "Rekordbox database path must name master.db: $source_path"
        exit 1
    fi
    if [[ ! -f "$source_path" ]]; then
        err "Rekordbox database file not found: $source_path"
        exit 1
    fi

    RB_DATA="$(canonical_parent_directory "$source_path")"
    EFFECTIVE_DB_PATH="$RB_DATA/master.db"
    if [[ ! "$source_path" -ef "$EFFECTIVE_DB_PATH" ]]; then
        err "Configured database does not resolve to the selected master.db: $source_path"
        exit 1
    fi
}

configure_restore_target() {
    local configured="${REKORDBOX_DB_PATH:-}"
    if [[ -z "$configured" ]]; then
        RB_DATA="$DEFAULT_RB_DATA"
        EFFECTIVE_DB_PATH="$RB_DATA/master.db"
    else
        if [[ -L "$configured" ]]; then
            err "Rekordbox database path must be direct; symlinks are not supported: $configured"
            exit 1
        fi
        if [[ "$(basename "$configured")" != "master.db" ]]; then
            err "Rekordbox database path must name master.db: $configured"
            exit 1
        fi
        if [[ -e "$configured" && ! -f "$configured" ]]; then
            err "Rekordbox database path is not a regular file: $configured"
            exit 1
        fi
        RB_DATA="$(canonical_parent_directory "$configured")"
        EFFECTIVE_DB_PATH="$RB_DATA/master.db"
    fi

    if [[ ! -d "$RB_DATA" ]]; then
        err "Rekordbox restore target directory not found: $RB_DATA"
        exit 1
    fi
}

check_rekordbox_running() {
    if pgrep -x rekordbox > /dev/null 2>&1; then
        warn "Rekordbox is currently running."
        warn "Backup will proceed but database files may be in a transient state."
        warn "For the most reliable backup, close Rekordbox first."
        echo ""
        return 1
    fi
    return 0
}

check_source_exists() {
    if [[ ! -d "$RB_DATA" ]]; then
        err "Rekordbox data directory not found: $RB_DATA"
        err "Is Rekordbox installed?"
        exit 1
    fi
    if [[ ! -f "$EFFECTIVE_DB_PATH" || -L "$EFFECTIVE_DB_PATH" ]]; then
        err "Rekordbox database source is not a direct regular master.db file: $EFFECTIVE_DB_PATH"
        exit 1
    fi
}

# --- Database files (the critical ones) ---
# These are the files that contain your library metadata, playlists,
# analysis state, and settings. This is what you CANNOT lose.
DB_FILES=(
    "master.db"
    "master.db-shm"
    "master.db-wal"
    "master.backup.db"
    "master.backup2.db"
    "master.backup3.db"
    "networkAnalyze6.db"
    "networkRecommend.db"
    "product.db"
    "datafile.edb"
    "datafile.backup.edb"
    "ExtData.edb"
    "ExtData.backup.edb"
    "masterPlaylists6.xml"
    "masterPlaylists3.xml"
    "automixPlaylist.xml"
    "automixPlaylist6.xml"
    "playlists3.sync"
)

# --- Functions ---

backup_db_only() {
    local label="${1:-db}"
    local archive="$BACKUP_DIR/${label}_${TIMESTAMP}.tar.gz"

    mkdir -p "$BACKUP_DIR"

    log "Backing up database files..."

    local files_to_backup=()
    for f in "${DB_FILES[@]}"; do
        if [[ -f "$RB_DATA/$f" ]]; then
            files_to_backup+=("$f")
        fi
    done

    if [[ ${#files_to_backup[@]} -eq 0 ]]; then
        err "No database files found to backup!"
        exit 1
    fi

    # Write to temp file, move on success (prevents partial archives on disk-full)
    local tmp_archive
    tmp_archive="$(mktemp "${BACKUP_DIR}/.backup_tmp.XXXXXX")"
    trap 'rm -f "$tmp_archive"; exit 1' INT TERM
    trap 'rm -f "$tmp_archive"' ERR

    tar -czf "$tmp_archive" -C "$RB_DATA" "${files_to_backup[@]}"
    mv "$tmp_archive" "$archive"
    trap - ERR INT TERM

    local size
    size=$(du -h "$archive" | cut -f1)
    log "Database backup created: $archive ($size)"
    echo "$archive"
}

backup_full() {
    local archive="$BACKUP_DIR/full_${TIMESTAMP}.tar.gz"

    mkdir -p "$BACKUP_DIR"

    log "Creating full backup (this may take a minute for ~1GB of data)..."

    # Write to temp file, move on success (prevents partial archives on disk-full)
    local tmp_archive
    tmp_archive="$(mktemp "${BACKUP_DIR}/.backup_tmp.XXXXXX")"
    trap 'rm -f "$tmp_archive"; exit 1' INT TERM
    trap 'rm -f "$tmp_archive"' ERR

    write_canonical_full_archive "$tmp_archive"
    mv "$tmp_archive" "$archive"
    trap - ERR INT TERM

    local size
    size=$(du -h "$archive" | cut -f1)
    log "Full backup created: $archive ($size)"
    echo "$archive"
}

write_canonical_full_archive() {
    local destination="$1"
    local backup_dir_canonical
    backup_dir_canonical="$(cd "$BACKUP_DIR" && pwd -P)"
    if [[ "$backup_dir_canonical" == "$RB_DATA" ]]; then
        err "Backup directory cannot be the Rekordbox data directory: $RB_DATA"
        return 1
    fi

    local -a archive_excludes=("--exclude=.DS_Store" "--exclude=*.tmp")
    if [[ "$backup_dir_canonical" == "$RB_DATA/"* ]]; then
        local nested_backup="${backup_dir_canonical#"$RB_DATA/"}"
        archive_excludes+=("--exclude=$nested_backup")
    fi
    local -a entries=()
    while IFS= read -r -d '' entry; do
        entries+=("${entry#./}")
    done < <(cd "$RB_DATA" && find . -mindepth 1 -maxdepth 1 -print0)
    if [[ ${#entries[@]} -eq 0 ]]; then
        err "Rekordbox data directory is empty: $RB_DATA"
        return 1
    fi
    local tar_version
    tar_version="$(tar --version 2>&1 || true)"
    if [[ "$tar_version" == *bsdtar* ]]; then
        tar -czf "$destination" \
            -C "$RB_DATA" \
            "${archive_excludes[@]}" \
            -s ',^,rekordbox/,' \
            -- \
            "${entries[@]}"
    else
        tar -czf "$destination" \
            -C "$RB_DATA" \
            "${archive_excludes[@]}" \
            --transform='s,^,rekordbox/,' \
            -- \
            "${entries[@]}"
    fi
}

list_backups() {
    if [[ ! -d "$BACKUP_DIR" ]]; then
        warn "No backups found. Run ./scripts/backup.sh to create one."
        return
    fi

    echo ""
    log "Existing backups in $BACKUP_DIR:"
    echo ""

    local total=0
    for f in "$BACKUP_DIR"/*.tar.gz; do
        [[ -f "$f" ]] || continue
        total=$((total + 1))
        local size
        size=$(du -h "$f" | cut -f1)
        local basename
        basename=$(basename "$f")

        # Parse type from filename
        local type="unknown"
        if [[ "$basename" == full_* ]]; then
            type="${BLUE}full${NC}   "
        elif [[ "$basename" == db_* ]]; then
            type="${GREEN}db${NC}     "
        elif [[ "$basename" == pre-op_* ]]; then
            type="${YELLOW}pre-op${NC} "
        fi

        echo -e "  $type  $size  $basename"
    done

    if [[ $total -eq 0 ]]; then
        warn "No backups found."
    else
        echo ""
        log "$total backup(s) found."
        local total_size
        total_size=$(du -sh "$BACKUP_DIR" | cut -f1)
        log "Total backup storage: $total_size"
    fi
}

rotate_backups() {
    local prefix="$1"
    local max="$2"

    local count
    count=$(ls "$BACKUP_DIR"/${prefix}_*.tar.gz 2>/dev/null | wc -l | tr -d ' ')

    if [[ "$count" -gt "$max" ]]; then
        local to_remove=$((count - max))
        log "Rotating: removing $to_remove old ${prefix} backup(s)..."
        ls -t "$BACKUP_DIR"/${prefix}_*.tar.gz | tail -n "$to_remove" | while read -r f; do
            log "  Removing $(basename "$f")"
            rm -f "$f"
        done
    fi
}

is_allowed_db_file() {
    local candidate="$1"
    local allowed
    for allowed in "${DB_FILES[@]}"; do
        if [[ "$allowed" == "$candidate" ]]; then
            return 0
        fi
    done
    return 1
}

restore_backup() {
    local archive="$1"

    if [[ ! -f "$archive" ]]; then
        err "Backup file not found: $archive"
        exit 1
    fi

    # Safety: check if rekordbox is running
    if pgrep -x rekordbox > /dev/null 2>&1; then
        err "Rekordbox is running! Close it before restoring."
        err "Restore aborted."
        exit 1
    fi

    local basename
    basename=$(basename "$archive")

    echo ""
    warn "=== RESTORE OPERATION ==="
    warn "Archive: $basename"
    warn "Target:  $RB_DATA"
    echo ""

    # Show what's in the archive
    log "Archive contents:"
    tar -tzf "$archive" | sed -n '1,20p'
    local file_count
    file_count=$(tar -tzf "$archive" | wc -l | tr -d ' ')
    if [[ "$file_count" -gt 20 ]]; then
        echo "  ... and $((file_count - 20)) more files"
    fi
    echo ""

    local restore_mode
    if [[ "$basename" == full_* ]]; then
        restore_mode="full"
        warn "This is a FULL restore. It will replace your entire Rekordbox data directory."
    else
        restore_mode="db"
        warn "This is a DB restore. It will replace database and config files only."
        warn "Analysis data and artwork will remain unchanged."
    fi
    echo ""

    read -rp "$(echo -e "${YELLOW}Are you sure you want to restore? (type YES to confirm): ${NC}")" confirm
    if [[ "$confirm" != "YES" ]]; then
        log "Restore cancelled."
        exit 0
    fi

    # Create a safety backup of current state before restoring.
    # Match the restore type so non-DB data is preserved on full restores.
    # The subshell isolates trap modifications from restore_backup's context.
    # The full_pre-restore_ prefix ensures the archive routes to the full
    # restore path (matching full_*) when later restored via --restore.
    log "Creating safety backup of current state..."
    if [[ "$restore_mode" == "full" ]]; then
        (
            safety_archive="$BACKUP_DIR/full_pre-restore_${TIMESTAMP}.tar.gz"
            mkdir -p "$BACKUP_DIR"
            tmp_archive="$(mktemp "${BACKUP_DIR}/.backup_tmp.XXXXXX")"
            trap 'rm -f "$tmp_archive"; exit 1' INT TERM
            trap 'rm -f "$tmp_archive"; err "Safety backup failed; your data is unchanged."' ERR
            write_canonical_full_archive "$tmp_archive"
            mv "$tmp_archive" "$safety_archive"
            trap - ERR INT TERM
            log "Full safety backup created: $safety_archive"
        )
    else
        local current_db_file=""
        local candidate
        for candidate in "${DB_FILES[@]}"; do
            if [[ -f "$RB_DATA/$candidate" ]]; then
                current_db_file="$RB_DATA/$candidate"
                break
            fi
        done
        if [[ -n "$current_db_file" ]]; then
            backup_db_only "pre-restore"
        else
            log "No current database files to back up; continuing restore."
        fi
    fi

    local staging_dir
    staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/reklawdbox-restore.XXXXXX")"
    if ! tar -xzf "$archive" -C "$staging_dir"; then
        err "Restore failed while extracting archive into staging."
        rm -rf "$staging_dir"
        exit 1
    fi

    if [[ "$restore_mode" == "full" ]]; then
        # Full restore: replace the entire directory with the staged snapshot.
        log "Restoring full backup..."

        local staged_rb_dir
        staged_rb_dir="$staging_dir/rekordbox"
        if [[ ! -d "$staged_rb_dir" ]]; then
            err "Full backup is missing expected top-level directory: rekordbox"
            rm -rf "$staging_dir"
            exit 1
        fi

        local unexpected_entry
        unexpected_entry="$(find "$staging_dir" -mindepth 1 -maxdepth 1 ! -name "rekordbox" -print -quit)"
        if [[ -n "$unexpected_entry" ]]; then
            err "Full backup contains unexpected top-level entry: $(basename "$unexpected_entry")"
            rm -rf "$staging_dir"
            exit 1
        fi

        local special_entry
        special_entry="$(find "$staged_rb_dir" \( -type l -o -type p -o -type b -o -type c -o -type s \) -print -quit)"
        if [[ -n "$special_entry" ]]; then
            err "Full backup contains unsupported file types (e.g. symlinks); refusing restore."
            rm -rf "$staging_dir"
            exit 1
        fi

        local rollback_dir
        rollback_dir="${RB_DATA}.restore-backup-${TIMESTAMP}"
        if ! mv "$RB_DATA" "$rollback_dir"; then
            err "Failed to move existing data aside for restore. Your data is unchanged."
            rm -rf "$staging_dir"
            exit 1
        fi
        if ! mv "$staged_rb_dir" "$RB_DATA"; then
            err "Failed to move restored data into place; attempting rollback."
            if ! mv "$rollback_dir" "$RB_DATA"; then
                err "Rollback failed. Previous data is at: $rollback_dir"
            fi
            rm -rf "$staging_dir"
            exit 1
        fi
        rm -rf "$rollback_dir"
    else
        # DB restore: apply only validated top-level DB files.
        log "Restoring database files..."

        local nested_entry
        nested_entry="$(find "$staging_dir" -mindepth 2 -print -quit)"
        if [[ -n "$nested_entry" ]]; then
            err "DB backup contains nested paths; refusing restore."
            rm -rf "$staging_dir"
            exit 1
        fi

        local special_entry
        special_entry="$(find "$staging_dir" -mindepth 1 \( -type l -o -type p -o -type b -o -type c -o -type s \) -print -quit)"
        if [[ -n "$special_entry" ]]; then
            err "DB backup contains unsupported file types; refusing restore."
            rm -rf "$staging_dir"
            exit 1
        fi

        local -a staged_files=()
        while IFS= read -r staged_file; do
            local file_name
            file_name="$(basename "$staged_file")"
            if ! is_allowed_db_file "$file_name"; then
                err "DB backup contains unexpected file: $file_name"
                rm -rf "$staging_dir"
                exit 1
            fi
            staged_files+=("$staged_file")
        done < <(find "$staging_dir" -mindepth 1 -maxdepth 1 -type f -print | sort)

        if [[ "${#staged_files[@]}" -eq 0 ]]; then
            err "DB backup contained no restorable files."
            rm -rf "$staging_dir"
            exit 1
        fi

        # Atomic swap: move existing files aside, copy new files in, then
        # clean up. If any step fails, roll back the originals.
        local -a backed_up=()
        local bak_suffix=".restore-bak-${TIMESTAMP}"

        local rename_failed=0
        for staged_file in "${staged_files[@]}"; do
            local file_name
            file_name="$(basename "$staged_file")"
            if [[ -f "$RB_DATA/$file_name" ]]; then
                if ! mv "$RB_DATA/$file_name" "$RB_DATA/${file_name}${bak_suffix}"; then
                    rename_failed=1
                    err "Failed to rename aside $file_name; rolling back."
                    break
                fi
                backed_up+=("$file_name")
            fi
        done

        if [[ "$rename_failed" -eq 1 ]]; then
            if [[ ${#backed_up[@]} -gt 0 ]]; then
                for file_name in "${backed_up[@]}"; do
                    mv "$RB_DATA/${file_name}${bak_suffix}" "$RB_DATA/$file_name" 2>/dev/null || true
                done
            fi
            rm -rf "$staging_dir"
            exit 1
        fi

        local copy_failed=0
        for staged_file in "${staged_files[@]}"; do
            local file_name
            file_name="$(basename "$staged_file")"
            if ! cp -f "$staged_file" "$RB_DATA/$file_name"; then
                copy_failed=1
                break
            fi
        done

        if [[ "$copy_failed" -eq 1 ]]; then
            err "Copy failed; rolling back DB restore."
            for staged_file in "${staged_files[@]}"; do
                local file_name
                file_name="$(basename "$staged_file")"
                rm -f "$RB_DATA/$file_name"
            done
            local rollback_ok=1
            if [[ ${#backed_up[@]} -gt 0 ]]; then
                for file_name in "${backed_up[@]}"; do
                    if ! mv "$RB_DATA/${file_name}${bak_suffix}" "$RB_DATA/$file_name"; then
                        err "Rollback failed for $file_name. Manual recovery needed:"
                        err "  mv '$RB_DATA/${file_name}${bak_suffix}' '$RB_DATA/$file_name'"
                        rollback_ok=0
                    fi
                done
            fi
            if [[ "$rollback_ok" -eq 0 ]]; then
                err "Some files could not be rolled back. Check the paths above."
            fi
            rm -rf "$staging_dir"
            exit 1
        fi

        # All copies succeeded — remove the backup copies
        if [[ ${#backed_up[@]} -gt 0 ]]; then
            for file_name in "${backed_up[@]}"; do
                rm -f "$RB_DATA/${file_name}${bak_suffix}"
            done
        fi
    fi
    rm -rf "$staging_dir"

    log "Restore complete!"
    log "Start Rekordbox to verify your library."
}

# --- Main ---

case "${1:-}" in
    --db-only)
        configure_backup_source
        check_source_exists
        check_rekordbox_running || true
        backup_db_only "db"
        rotate_backups "db" "$MAX_DB_BACKUPS"
        ;;
    --pre-op)
        # Silent pre-operation backup (called by reklawdbox tools)
        configure_backup_source
        check_source_exists
        check_rekordbox_running || true
        backup_db_only "pre-op" > /dev/null
        rotate_backups "pre-op" "$MAX_DB_BACKUPS" > /dev/null
        ;;
    --list)
        list_backups
        ;;
    --restore)
        if [[ -z "${2:-}" ]]; then
            err "Usage: ./scripts/backup.sh --restore <path-to-backup.tar.gz>"
            echo ""
            list_backups
            exit 1
        fi
        configure_restore_target
        restore_backup "$2"
        ;;
    --help|-h)
        echo "reklawdbox: Rekordbox library backup tool"
        echo ""
        echo "Usage:"
        echo "  reklawdbox backup                  Full backup (~1GB compressed)"
        echo "  reklawdbox backup --db-only        Database files only (~50MB)"
        echo "  reklawdbox backup --list           List existing backups"
        echo "  reklawdbox backup --restore <path> Restore from backup"
        echo "  reklawdbox backup --help           Show this help"
        echo ""
        echo "Backups stored in: $BACKUP_DIR"
        ;;
    "")
        configure_backup_source
        check_source_exists
        check_rekordbox_running || true
        backup_full
        rotate_backups "full" "$MAX_FULL_BACKUPS"
        ;;
    *)
        err "Unknown option: $1"
        err "Run ./scripts/backup.sh --help for usage."
        exit 1
        ;;
esac
