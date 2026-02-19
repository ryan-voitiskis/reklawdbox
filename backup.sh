#!/bin/bash
# reklawdbox: Rekordbox library backup tool
# Snapshots the full Rekordbox library state to timestamped archives.
#
# Usage:
#   ./backup.sh                  # Full backup
#   ./backup.sh --db-only        # Database files only (fast, ~50MB)
#   ./backup.sh --pre-op         # Pre-operation snapshot (db-only, auto-called by tools)
#   ./backup.sh --list           # List existing backups
#   ./backup.sh --restore <path> # Restore from a backup archive
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
        warn "No backups found. Run ./backup.sh to create one."
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

    if [[ "$basename" == full_* ]]; then
        warn "This is a FULL restore. It will replace your entire Rekordbox data directory."
    else
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

    if [[ "$basename" == full_* ]]; then
        # Full restore: replace entire directory
        log "Restoring full backup..."
        # The full backup contains the 'rekordbox' directory itself
        tar -xzf "$archive" -C "$(dirname "$RB_DATA")"
    else
        # DB restore: extract into the rekordbox directory
        log "Restoring database files..."
        tar -xzf "$archive" -C "$RB_DATA"
    fi

    log "Restore complete!"
    log "Start Rekordbox to verify your library."
}

# --- Main ---

check_source_exists

case "${1:-}" in
    --db-only)
        check_rekordbox_running || true
        backup_db_only "db"
        rotate_backups "db" "$MAX_DB_BACKUPS"
        ;;
    --pre-op)
        # Silent pre-operation backup (called by reklawdbox tools)
        check_rekordbox_running || true
        backup_db_only "pre-op" > /dev/null
        rotate_backups "pre-op" "$MAX_DB_BACKUPS"
        ;;
    --list)
        list_backups
        ;;
    --restore)
        if [[ -z "${2:-}" ]]; then
            err "Usage: ./backup.sh --restore <path-to-backup.tar.gz>"
            echo ""
            list_backups
            exit 1
        fi
        restore_backup "$2"
        ;;
    --help|-h)
        echo "reklawdbox: Rekordbox library backup tool"
        echo ""
        echo "Usage:"
        echo "  ./backup.sh                  Full backup (~1GB compressed)"
        echo "  ./backup.sh --db-only        Database files only (~50MB)"
        echo "  ./backup.sh --pre-op         Pre-operation snapshot (silent)"
        echo "  ./backup.sh --list           List existing backups"
        echo "  ./backup.sh --restore <path> Restore from backup"
        echo "  ./backup.sh --help           Show this help"
        echo ""
        echo "Backups stored in: $BACKUP_DIR"
        ;;
    "")
        check_rekordbox_running || true
        backup_full
        rotate_backups "full" "$MAX_FULL_BACKUPS"
        ;;
    *)
        err "Unknown option: $1"
        err "Run ./backup.sh --help for usage."
        exit 1
        ;;
esac
