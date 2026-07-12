#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

stress_iterations=20
live_canary=false
live_max_tracks=5
live_playlist=""
run_release_gate=true

usage() {
  cat <<'EOF'
Usage: scripts/test-release-candidate.sh [options]

Runs the release gate, deterministic stress tests, and optional read-only
live-library canaries.

Options:
  --stress-iterations N  Number of focused stress repetitions (default: 20)
  --live-canary          Analyze a bounded selection from an explicit DB path
  --live-only            Run only the live canary (implies --live-canary)
  --live-max-tracks N    Maximum tracks for the live canary (default: 5)
  --live-playlist ID     Also run the DB-backed MCP smoke against this playlist
  -h, --help             Show this help

Live-canary safety:
  REKORDBOX_DB_PATH must be set to an existing regular file. If
  REKORDBOX_ANLZ_ROOT is set, it must be an existing directory. The command
  uses a temporary HOME and internal store, never invokes tag-write commands,
  and verifies the Rekordbox database hash and file identity afterward.
EOF
}

require_positive_integer() {
  local label="$1"
  local value="$2"
  if [[ ! "$value" =~ ^[1-9][0-9]*$ ]]; then
    echo "$label must be a positive integer: $value" >&2
    exit 2
  fi
}

while (($# > 0)); do
  case "$1" in
    --stress-iterations)
      [[ $# -ge 2 ]] || { echo "--stress-iterations requires a value" >&2; exit 2; }
      stress_iterations="$2"
      shift 2
      ;;
    --live-canary)
      live_canary=true
      shift
      ;;
    --live-only)
      live_canary=true
      run_release_gate=false
      shift
      ;;
    --live-max-tracks)
      [[ $# -ge 2 ]] || { echo "--live-max-tracks requires a value" >&2; exit 2; }
      live_max_tracks="$2"
      shift 2
      ;;
    --live-playlist)
      [[ $# -ge 2 ]] || { echo "--live-playlist requires a value" >&2; exit 2; }
      live_playlist="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

require_positive_integer "--stress-iterations" "$stress_iterations"
require_positive_integer "--live-max-tracks" "$live_max_tracks"

if [[ "$run_release_gate" == true ]]; then
  echo "==> Release gate"
  cargo fmt --check
  dprint check
  (
    cd site
    npm ci
    npm run build
  )
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace --no-fail-fast
  cargo build --release
  ./target/release/reklawdbox --version
  ./target/release/reklawdbox --help >/dev/null
  node scripts/mcp-smoke.mjs \
    --bin ./target/release/reklawdbox \
    --skip-db \
    --timeout-ms 60000

  echo "==> Serialized workspace test run"
  cargo test --workspace --no-fail-fast -- --test-threads=1

  echo "==> Focused stress tests ($stress_iterations iterations)"
  for ((iteration = 1; iteration <= stress_iterations; iteration += 1)); do
    echo "stress iteration $iteration/$stress_iterations"
    cargo test -p reklawdbox change_snapshot_guard
    cargo test -p reklawdbox write_xml
    cargo test -p reklawdbox audio_file_mutation
    cargo test -p reklawdbox discogs_auth
    cargo test -p reklawdbox cache_writer
    cargo test -p stratum-dsp --test integration_tests
  done
fi

if [[ "$live_canary" == true ]]; then
  : "${REKORDBOX_DB_PATH:?--live-canary requires REKORDBOX_DB_PATH}"
  if [[ ! -f "$REKORDBOX_DB_PATH" ]]; then
    echo "REKORDBOX_DB_PATH is not an existing regular file: $REKORDBOX_DB_PATH" >&2
    exit 2
  fi
  if [[ -n "${REKORDBOX_ANLZ_ROOT:-}" && ! -d "$REKORDBOX_ANLZ_ROOT" ]]; then
    echo "REKORDBOX_ANLZ_ROOT is not an existing directory: $REKORDBOX_ANLZ_ROOT" >&2
    exit 2
  fi

  canary_dir="$(mktemp -d "${TMPDIR:-/tmp}/reklawdbox-canary.XXXXXX")"
  trap 'rm -rf "$canary_dir"' EXIT
  mkdir -p "$canary_dir/home"

  db_hash_before="$(shasum -a 256 "$REKORDBOX_DB_PATH")"
  db_stat_before="$(stat -f '%d:%i:%z:%m' "$REKORDBOX_DB_PATH")"

  echo "==> Read-only live-library canary ($live_max_tracks tracks maximum)"
  HOME="$canary_dir/home" \
    CRATE_DIG_STORE_PATH="$canary_dir/internal.sqlite3" \
    REKORDBOX_DB_PATH="$REKORDBOX_DB_PATH" \
    ./target/release/reklawdbox analyze \
      --max-tracks "$live_max_tracks" \
      --stratum-only \
      --concurrency 1

  cli_store="$canary_dir/internal.sqlite3"
  if [[ ! -f "$cli_store" ]]; then
    echo "live canary did not create its isolated CLI cache: $cli_store" >&2
    exit 1
  fi
  cached_before="$(sqlite3 "$cli_store" "SELECT count(*) FROM audio_analysis_cache WHERE analyzer = 'stratum-dsp';")"
  missing_fingerprints="$(sqlite3 "$cli_store" "SELECT count(*) FROM audio_analysis_cache WHERE analyzer = 'stratum-dsp' AND input_fingerprint = '';")"
  if ((cached_before < 1 || cached_before > live_max_tracks)); then
    echo "unexpected live-canary cache row count: $cached_before" >&2
    exit 1
  fi
  if ((missing_fingerprints != 0)); then
    echo "live canary created Stratum rows without input fingerprints" >&2
    exit 1
  fi

  echo "==> Live-library cache-reuse canary"
  HOME="$canary_dir/home" \
    CRATE_DIG_STORE_PATH="$canary_dir/internal.sqlite3" \
    REKORDBOX_DB_PATH="$REKORDBOX_DB_PATH" \
    ./target/release/reklawdbox analyze \
      --max-tracks "$live_max_tracks" \
      --stratum-only \
      --concurrency 1
  cached_after="$(sqlite3 "$cli_store" "SELECT count(*) FROM audio_analysis_cache WHERE analyzer = 'stratum-dsp';")"
  if [[ "$cached_before" != "$cached_after" ]]; then
    echo "cache-reuse canary unexpectedly changed the Stratum row count" >&2
    exit 1
  fi

  if [[ -n "$live_playlist" ]]; then
    echo "==> DB-backed MCP smoke for playlist $live_playlist"
    HOME="$canary_dir/home" \
      CRATE_DIG_STORE_PATH="$canary_dir/internal.sqlite3" \
      REKORDBOX_DB_PATH="$REKORDBOX_DB_PATH" \
      node scripts/mcp-smoke.mjs \
        --bin ./target/release/reklawdbox \
        --playlist "$live_playlist" \
        --timeout-ms 60000
  fi

  db_hash_after="$(shasum -a 256 "$REKORDBOX_DB_PATH")"
  db_stat_after="$(stat -f '%d:%i:%z:%m' "$REKORDBOX_DB_PATH")"
  if [[ "$db_hash_before" != "$db_hash_after" || "$db_stat_before" != "$db_stat_after" ]]; then
    echo "Rekordbox database changed during the live canary" >&2
    exit 1
  fi
fi

git diff --check
echo "==> Release-candidate checks passed"
