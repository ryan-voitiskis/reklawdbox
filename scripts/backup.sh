#!/bin/bash
# reklawdbox: Rekordbox library backup tool
# Snapshots the full Rekordbox library state to timestamped archives.
#
# Usage:
#   ./scripts/backup.sh                  # Full backup
#   ./scripts/backup.sh --db-only        # Database files only (fast, ~50MB)
#   ./scripts/backup.sh --pre-op         # Pre-operation snapshot (db-only, auto-called by tools)
#   ./scripts/backup.sh --list           # List existing backups
#   ./scripts/backup.sh --restore <path> # Restore from a backup archive
#
# Backup location: ~/Library/Pioneer/rekordbox-backups/

set -euo pipefail

# --- Configuration ---
RB_DATA="$HOME/Library/Pioneer/rekordbox"
BACKUP_DIR="$HOME/Library/Pioneer/rekordbox-backups"
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

    # Use tar from the RB_DATA directory
    tar -czf "$archive" -C "$RB_DATA" "${files_to_backup[@]}"

    local size
    size=$(du -h "$archive" | cut -f1)
    log "Database backup created: $archive ($size)"
    echo "$archive"
}

backup_full() {
    local archive="$BACKUP_DIR/full_${TIMESTAMP}.tar.gz"

    mkdir -p "$BACKUP_DIR"

    log "Creating full backup (this may take a minute for ~1GB of data)..."

    # Exclude macOS metadata and any temp files
    tar -czf "$archive" \
        -C "$(dirname "$RB_DATA")" \
        --exclude='.DS_Store' \
        --exclude='*.tmp' \
        "$(basename "$RB_DATA")"

    local size
    size=$(du -h "$archive" | cut -f1)
    log "Full backup created: $archive ($size)"
    echo "$archive"
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
            rm "$f"
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
    tar -tzf "$archive" | head -20
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

    # Create a safety backup of current state before restoring
    log "Creating safety backup of current state..."
    backup_db_only "pre-restore"

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

        local rb_name
        rb_name="$(basename "$RB_DATA")"
        local staged_rb_dir
        staged_rb_dir="$staging_dir/$rb_name"
        if [[ ! -d "$staged_rb_dir" ]]; then
            err "Full backup is missing expected top-level directory: $rb_name"
            rm -rf "$staging_dir"
            exit 1
        fi

        local unexpected_entry
        unexpected_entry="$(find "$staging_dir" -mindepth 1 -maxdepth 1 ! -name "$rb_name" -print -quit)"
        if [[ -n "$unexpected_entry" ]]; then
            err "Full backup contains unexpected top-level entry: $(basename "$unexpected_entry")"
            rm -rf "$staging_dir"
            exit 1
        fi

        local rollback_dir
        rollback_dir="${RB_DATA}.restore-backup-${TIMESTAMP}"
        mv "$RB_DATA" "$rollback_dir"
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

        local staged_file
        for staged_file in "${staged_files[@]}"; do
            local file_name
            file_name="$(basename "$staged_file")"
            cp -f "$staged_file" "$RB_DATA/$file_name"
        done
    fi
    rm -rf "$staging_dir"

    log "Restore complete!"
    log "Start Rekordbox to verify your library."
}

# --- Main ---

case "${1:-}" in
    --db-only)
        check_source_exists
        check_rekordbox_running || true
        backup_db_only "db"
        rotate_backups "db" "$MAX_DB_BACKUPS"
        ;;
    --pre-op)
        # Silent pre-operation backup (called by reklawdbox tools)
        check_source_exists
        check_rekordbox_running || true
        backup_db_only "pre-op" > /dev/null
        rotate_backups "pre-op" "$MAX_DB_BACKUPS"
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
        check_source_exists
        restore_backup "$2"
        ;;
    --help|-h)
        echo "reklawdbox: Rekordbox library backup tool"
        echo ""
        echo "Usage:"
        echo "  ./scripts/backup.sh                  Full backup (~1GB compressed)"
        echo "  ./scripts/backup.sh --db-only        Database files only (~50MB)"
        echo "  ./scripts/backup.sh --pre-op         Pre-operation snapshot (silent)"
        echo "  ./scripts/backup.sh --list           List existing backups"
        echo "  ./scripts/backup.sh --restore <path> Restore from backup"
        echo "  ./scripts/backup.sh --help           Show this help"
        echo ""
        echo "Backups stored in: $BACKUP_DIR"
        ;;
    "")
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
