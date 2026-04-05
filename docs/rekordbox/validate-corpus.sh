#!/usr/bin/env bash

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS_DIR="$SCRIPT_DIR"

TARGET_DIRS=(manual guides faq features reference)

ALLOWED_TYPES=(
  manual
  guide
  faq
  feature
  reference
)

ALLOWED_MODES=(
  common
  export
  performance
  lighting
  edit
)

ALLOWED_CONFIDENCE=(
  pending
  verified
  high
)

# Controlled vocabulary for frontmatter topics
ALLOWED_TOPICS=(
  analysis
  backup
  beatgrid
  browsing
  cloud
  collaborative-playlists
  collection
  color
  comments
  compatibility
  connection
  cue-points
  devices
  dvs
  edit
  effects
  equipment
  export
  file-formats
  genre
  history
  hot-cue
  import
  interface
  key
  library
  lighting
  link
  memory-cue
  metadata
  midi
  mixing
  mobile
  onelibrary
  pads
  performance
  phrase
  playback
  playlists
  preferences
  pro-dj-link
  rating
  recording
  sampler
  search
  sequencer
  slicer
  stems
  streaming
  subscription
  system-requirements
  track-suggestion
  usb
  video
  waveform
  xml
)

MANUAL_EXPECTED_MIN="${MANUAL_EXPECTED_MIN:-1}"
MANUAL_EXPECTED_MAX="${MANUAL_EXPECTED_MAX:-259}"
# Optional comma-separated list of pages to ignore in coverage checks.
MANUAL_IGNORE_PAGES="${MANUAL_IGNORE_PAGES:-}"

ERRORS=0
WARNINGS=0

TMP_DIR="$(mktemp -d /tmp/rekordbox-corpus-validate.XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

TMP_IDS="$TMP_DIR/ids.tsv"
TMP_MANUAL_PAGES="$TMP_DIR/manual-pages.tsv"
TMP_ALLOWED_TOPICS="$TMP_DIR/allowed-topics.txt"
TMP_ALLOWED_TYPES="$TMP_DIR/allowed-types.txt"
TMP_ALLOWED_MODES="$TMP_DIR/allowed-modes.txt"
TMP_ALLOWED_CONF="$TMP_DIR/allowed-confidence.txt"
TMP_IGNORE_PAGES="$TMP_DIR/ignore-pages.txt"

touch "$TMP_IDS" "$TMP_MANUAL_PAGES" "$TMP_IGNORE_PAGES"

printf '%s\n' "${ALLOWED_TOPICS[@]}" | sort -u > "$TMP_ALLOWED_TOPICS"
printf '%s\n' "${ALLOWED_TYPES[@]}" | sort -u > "$TMP_ALLOWED_TYPES"
printf '%s\n' "${ALLOWED_MODES[@]}" | sort -u > "$TMP_ALLOWED_MODES"
printf '%s\n' "${ALLOWED_CONFIDENCE[@]}" | sort -u > "$TMP_ALLOWED_CONF"

trim() {
  printf '%s' "$1" | sed -E 's/^[[:space:]]+|[[:space:]]+$//g'
}

lower() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

strip_quotes() {
  local value="$1"
  value="$(trim "$value")"
  value="${value#\"}"
  value="${value%\"}"
  printf '%s' "$value"
}

contains_line() {
  local needle="$1"
  local file="$2"
  grep -Fxq "$needle" "$file"
}

err() {
  ERRORS=$((ERRORS + 1))
  printf 'ERROR: %s\n' "$1"
}

warn() {
  WARNINGS=$((WARNINGS + 1))
  printf 'WARN: %s\n' "$1"
}

extract_frontmatter() {
  local file="$1"
  awk '
    NR == 1 && $0 == "---" { in_fm = 1; next }
    in_fm && $0 == "---" { exit }
    in_fm { print }
  ' "$file"
}

expand_pages() {
  local pages_raw="$1"
  local pages
  local chunk
  local start
  local end
  local i

  pages="$(printf '%s' "$pages_raw" | tr -d '[:space:]')"
  IFS=',' read -r -a chunks <<< "$pages"

  for chunk in "${chunks[@]}"; do
    [[ -z "$chunk" ]] && continue
    if [[ "$chunk" =~ ^[0-9]+-[0-9]+$ ]]; then
      start="${chunk%-*}"
      end="${chunk#*-}"
      if (( start > end )); then
        return 1
      fi
      for (( i = start; i <= end; i++ )); do
        printf '%s\n' "$i"
      done
    elif [[ "$chunk" =~ ^[0-9]+$ ]]; then
      printf '%s\n' "$chunk"
    else
      return 1
    fi
  done
}

format_ranges() {
  local -a values=("$@")
  local out=""
  local i
  local start
  local prev
  local current

  [[ ${#values[@]} -eq 0 ]] && return 0

  start="${values[0]}"
  prev="${values[0]}"

  for (( i = 1; i < ${#values[@]}; i++ )); do
    current="${values[$i]}"
    if (( current == prev + 1 )); then
      prev="$current"
      continue
    fi

    if [[ -n "$out" ]]; then
      out+=", "
    fi
    if (( start == prev )); then
      out+="$start"
    else
      out+="$start-$prev"
    fi

    start="$current"
    prev="$current"
  done

  if [[ -n "$out" ]]; then
    out+=", "
  fi
  if (( start == prev )); then
    out+="$start"
  else
    out+="$start-$prev"
  fi

  printf '%s\n' "$out"
}

if [[ -n "$MANUAL_IGNORE_PAGES" ]]; then
  printf '%s\n' "$MANUAL_IGNORE_PAGES" | tr ',' '\n' | sed -E 's/^[[:space:]]+|[[:space:]]+$//g' | awk 'NF' | sort -n -u > "$TMP_IGNORE_PAGES"
fi

FILES=()
for dir in "${TARGET_DIRS[@]}"; do
  if [[ -d "$CORPUS_DIR/$dir" ]]; then
    while IFS= read -r -d '' file; do
      FILES+=("$file")
    done < <(find "$CORPUS_DIR/$dir" -type f -name '*.md' -print0)
  fi
done

if [[ ${#FILES[@]} -eq 0 ]]; then
  err "No markdown files found under ${TARGET_DIRS[*]} in $CORPUS_DIR"
  exit 1
fi

printf 'Scanning %d documents in %s\n' "${#FILES[@]}" "$CORPUS_DIR"

for file in "${FILES[@]}"; do
  rel="${file#$CORPUS_DIR/}"
  first_line="$(sed -n '1p' "$file")"
  if [[ "$first_line" != "---" ]]; then
    err "$rel: missing frontmatter opening delimiter ('---') on line 1"
    continue
  fi

  if ! awk 'NR > 1 && $0 == "---" { found = 1; exit } END { exit(found ? 0 : 1) }' "$file"; then
    err "$rel: missing frontmatter closing delimiter ('---')"
    continue
  fi

  fm="$(extract_frontmatter "$file")"

  for key in id title type source topics modes confidence last_verified transcribed_by verified_by; do
    if ! printf '%s\n' "$fm" | grep -q "^${key}:"; then
      err "$rel: missing required frontmatter key '${key}'"
    fi
  done

  id_raw="$(printf '%s\n' "$fm" | sed -n 's/^id:[[:space:]]*//p' | head -n1)"
  id="$(strip_quotes "$id_raw")"
  if [[ -n "$id" ]]; then
    printf '%s\t%s\n' "$id" "$rel" >> "$TMP_IDS"
  fi

  type_raw="$(printf '%s\n' "$fm" | sed -n 's/^type:[[:space:]]*//p' | head -n1)"
  type="$(lower "$(strip_quotes "$type_raw")")"
  if [[ -n "$type" ]] && ! contains_line "$type" "$TMP_ALLOWED_TYPES"; then
    err "$rel: invalid type '$type_raw' (allowed: ${ALLOWED_TYPES[*]})"
  fi

  source_file_raw="$(printf '%s\n' "$fm" | sed -n 's/^  file:[[:space:]]*//p' | head -n1)"
  source_url_raw="$(printf '%s\n' "$fm" | sed -n 's/^  url:[[:space:]]*//p' | head -n1)"
  source_pages_raw="$(printf '%s\n' "$fm" | sed -n 's/^  pages:[[:space:]]*//p' | head -n1)"
  source_version_raw="$(printf '%s\n' "$fm" | sed -n 's/^  version:[[:space:]]*//p' | head -n1)"

  source_file="$(strip_quotes "$source_file_raw")"
  source_url="$(strip_quotes "$source_url_raw")"
  source_pages="$(strip_quotes "$source_pages_raw")"
  source_version="$(strip_quotes "$source_version_raw")"

  if [[ -z "$source_version" ]]; then
    err "$rel: source metadata missing 'version'"
  fi

  if [[ -z "$source_file" && -z "$source_url" ]]; then
    err "$rel: source metadata must include at least one of 'file' or 'url'"
  fi

  if [[ "$type" == "manual" ]]; then
    if [[ -z "$source_file" ]]; then
      err "$rel: manual docs must include source.file"
    fi
    if [[ -z "$source_pages" ]]; then
      err "$rel: manual docs must include source.pages"
    fi
  fi

  if [[ "$type" == "guide" && -n "$source_file" && "$source_file" == *.pdf && -z "$source_pages" ]]; then
    err "$rel: guide sourced from PDF must include source.pages"
  fi

  if [[ "$type" == "faq" ]]; then
    if [[ -z "$source_file" ]]; then
      err "$rel: faq docs must include source.file"
    fi
    if [[ -z "$source_url" ]]; then
      err "$rel: faq docs must include source.url"
    fi
  fi

  if [[ "$type" == "feature" || "$type" == "reference" ]]; then
    if [[ -z "$source_url" ]]; then
      err "$rel: ${type} docs should include source.url"
    fi
  fi

  conf_raw="$(printf '%s\n' "$fm" | sed -n 's/^confidence:[[:space:]]*//p' | head -n1)"
  conf="$(lower "$(strip_quotes "$conf_raw")")"
  if [[ -n "$conf" ]] && ! contains_line "$conf" "$TMP_ALLOWED_CONF"; then
    err "$rel: invalid confidence '$conf_raw' (allowed: ${ALLOWED_CONFIDENCE[*]})"
  fi

  modes_line="$(printf '%s\n' "$fm" | sed -n 's/^modes:[[:space:]]*\[\(.*\)\][[:space:]]*$/\1/p' | head -n1)"
  if [[ -z "$modes_line" ]]; then
    err "$rel: unable to parse modes list"
  else
    IFS=',' read -r -a modes <<< "$modes_line"
    for mode_item in "${modes[@]}"; do
      mode_trimmed="$(trim "$mode_item")"
      [[ -z "$mode_trimmed" ]] && continue
      mode_norm="$(lower "$mode_trimmed")"
      if ! contains_line "$mode_norm" "$TMP_ALLOWED_MODES"; then
        err "$rel: invalid mode '$mode_trimmed' (allowed: ${ALLOWED_MODES[*]})"
      fi
      if [[ "$mode_trimmed" != "$mode_norm" ]]; then
        warn "$rel: mode '$mode_trimmed' should be normalized to '$mode_norm'"
      fi
    done
  fi

  topics_line="$(printf '%s\n' "$fm" | sed -n 's/^topics:[[:space:]]*\[\(.*\)\][[:space:]]*$/\1/p' | head -n1)"
  if [[ -z "$topics_line" ]]; then
    err "$rel: unable to parse topics list"
  else
    IFS=',' read -r -a topics <<< "$topics_line"
    for topic_item in "${topics[@]}"; do
      topic_trimmed="$(trim "$topic_item")"
      [[ -z "$topic_trimmed" ]] && continue
      topic_norm="$(lower "$topic_trimmed")"
      if ! contains_line "$topic_norm" "$TMP_ALLOWED_TOPICS"; then
        err "$rel: topic '$topic_trimmed' is outside controlled vocabulary"
      fi
      if [[ "$topic_trimmed" != "$topic_norm" ]]; then
        warn "$rel: topic '$topic_trimmed' should be normalized to '$topic_norm'"
      fi
    done
  fi

  if grep -Eiq 'AGENT NOTE|next agent|continue from page' "$file"; then
    err "$rel: unresolved agent handoff marker found in document body"
  fi

  if [[ "$type" == "manual" ]]; then
    if [[ -z "$source_pages" ]]; then
      err "$rel: manual document is missing source.pages value"
    else
      if ! expanded="$(expand_pages "$source_pages")"; then
        err "$rel: invalid source.pages format '$source_pages'"
      else
        while IFS= read -r page; do
          [[ -z "$page" ]] && continue
          printf '%s\t%s\n' "$page" "$rel" >> "$TMP_MANUAL_PAGES"
        done <<< "$expanded"
      fi
    fi
  fi
done

if [[ -s "$TMP_IDS" ]]; then
  while IFS= read -r dup_id; do
    [[ -z "$dup_id" ]] && continue
    err "duplicate id '$dup_id' found in:"
    grep -E "^${dup_id}[[:space:]]" "$TMP_IDS" | cut -f2 | sed 's/^/  - /'
  done < <(cut -f1 "$TMP_IDS" | sort | uniq -d)
fi

if [[ -s "$TMP_MANUAL_PAGES" ]]; then
  TMP_COVERED="$TMP_DIR/manual-covered.txt"
  cut -f1 "$TMP_MANUAL_PAGES" | sort -n -u > "$TMP_COVERED"

  missing_pages=()
  for (( page = MANUAL_EXPECTED_MIN; page <= MANUAL_EXPECTED_MAX; page++ )); do
    if contains_line "$page" "$TMP_IGNORE_PAGES"; then
      continue
    fi
    if ! contains_line "$page" "$TMP_COVERED"; then
      missing_pages+=("$page")
    fi
  done

  if [[ ${#missing_pages[@]} -gt 0 ]]; then
    ranges="$(format_ranges "${missing_pages[@]}")"
    err "manual page coverage gap(s) detected in ${MANUAL_EXPECTED_MIN}-${MANUAL_EXPECTED_MAX}: ${ranges}"
    if [[ -s "$TMP_IGNORE_PAGES" ]]; then
      warn "manual coverage ignore list is active: ${MANUAL_IGNORE_PAGES}"
    fi
  fi

  while IFS= read -r overlap_page; do
    [[ -z "$overlap_page" ]] && continue
    warn "manual page ${overlap_page} appears in multiple files:"
    awk -F'\t' -v p="$overlap_page" '$1 == p { print "  - " $2 }' "$TMP_MANUAL_PAGES" | sort -u
  done < <(cut -f1 "$TMP_MANUAL_PAGES" | sort -n | uniq -d)
fi

if [[ ! -f "$CORPUS_DIR/manifest.yaml" ]]; then
  warn "manifest.yaml is missing"
fi

if [[ ! -f "$CORPUS_DIR/README.md" ]]; then
  warn "README.md is missing in docs/rekordbox/"
fi

for dir in "${TARGET_DIRS[@]}"; do
  if [[ -d "$CORPUS_DIR/$dir" ]]; then
    count="$(find "$CORPUS_DIR/$dir" -type f -name '*.md' | wc -l | tr -d ' ')"
    printf 'Count %-10s %s\n' "$dir:" "$count"
    if [[ "$count" == "0" ]]; then
      warn "$dir/ is currently empty"
    fi
  fi
done

printf '\nValidation summary: %d error(s), %d warning(s)\n' "$ERRORS" "$WARNINGS"

if (( ERRORS > 0 )); then
  exit 1
fi

exit 0
