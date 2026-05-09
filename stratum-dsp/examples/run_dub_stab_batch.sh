#!/usr/bin/env bash
# Run dub_stab_real_audio over all Dub Techno tracks in genre_verified.
# Tracks listed as "<bpm>\t<path>" pairs; one line per track.
# Output streams full per-track diagnostics; SUMMARY lines are grep-friendly.
#
# Set GRID_JSON to the output of `scripts/dump_rekordbox_grids.py` to use
# Rekordbox's hand-verified beat grid instead of the HMM tracker.

set -euo pipefail

BIN="$(dirname "$0")/../../target/release/examples/dub_stab_real_audio"
GRID_JSON="${GRID_JSON:-}"

while IFS=$'\t' read -r bpm path; do
    [ -z "$bpm" ] && continue
    if [ -n "$GRID_JSON" ]; then
        "$BIN" "$path" "$bpm" "$GRID_JSON" || echo "FAILED: $path"
    else
        "$BIN" "$path" "$bpm" || echo "FAILED: $path"
    fi
    echo
done <<'EOF'
122	/Users/vz/Music/play/play5/E110 - 00946.wav
126	/Users/vz/Music/play/play5/E110 - After Irradiation.wav
125	/Users/vz/Music/collection/Gradient/Autumn Clouds EP (2021)/01 Gradient - Cloud One.wav
127	/Users/vz/Music/collection/Gradient/Autumn Clouds EP (2021)/02 Submoon, Gradient - Cloud One (Submoon RMX).wav
120	/Users/vz/Music/collection/Gradient/Autumn Clouds EP (2021)/04 Gradient - Cloud Three.wav
123	/Users/vz/Music/collection/Gradient/Autumn Clouds EP (2021)/03 Gradient - Cloud Two.wav
115	/Users/vz/Music/play/play5/E110 - Dubster.wav
120	/Users/vz/Music/play/play32/Burger Ink - Elvism.wav
116.54	/Users/vz/Music/collection/Monolake/Momentum (2003)/06 Monolake - Excentric.flac
123.85	/Users/vz/Music/collection/Dub Taylor/Forms & Figures (2001)/11 Dub Taylor - Figure Two (Original 12" Version).flac
120	/Users/vz/Music/collection/Dub Taylor/Forms & Figures (2001)/17 Dub Taylor, Phew - Generate Dub (Bonus Track).flac
125	/Users/vz/Music/collection/Various Artists/Fachwerk/Fachwerk Part 3 (2019)/11 Sascha Rydell - Laisser Faire.flac
122.4	/Users/vz/Music/play/play32/Burger Ink - Love Is the Drug [Paris Texas].wav
126	/Users/vz/Music/play/play5/E110 - My Last Love.wav
124	/Users/vz/Music/play/play32/Azuni - Orkus.wav
120	/Users/vz/Music/play/play9/Vladislav Delay - Recovery IDea (Andy Stott Remix).wav
130	/Users/vz/Music/collection/Monolake/Momentum (2003)/07 Monolake - Reminiscence.flac
127	/Users/vz/Music/play/play5/E110 - SKFS.wav
125	/Users/vz/Music/play/play32/Donato Dozzy, Peter Van Hoesen - Talis.wav
130	/Users/vz/Music/collection/Dialogue/dialogical (2020)/02 Dialogue - Talkstart.wav
125	/Users/vz/Music/play/play5/E110 - Track 3.wav
119	/Users/vz/Music/play/play5/E110 - Unheard Bedtime Story.wav
130	/Users/vz/Music/play/play5/E110 - Untitled 2.wav
123	/Users/vz/Music/play/play24/Waage - W7.wav
EOF
