# Real-audio benchmark v1

This benchmark freezes the 24 Dub Techno tracks used by the original chord-stab
investigation. It is a private-corpus regression benchmark: the repository
contains the manifest, listening annotations, comparison tolerances, and
baseline outputs, but not copyrighted audio or Rekordbox beat-grid exports.

The goal is to detect accidental DSP drift while feature extraction and genre
classification evolve. It is not an accuracy score for the classifier: all 24
tracks are positive examples from `genre_verified`, so there are no controls or
negative classes.

## Run

Create a Rekordbox grid export for the manifest's track IDs using
`scripts/dump_rekordbox_grids.py`, then run:

```bash
export REKLAW_AUDIO_ROOT="$HOME/Music"
export REKLAW_GRID_JSON=/private/path/real-audio-v1-grids.json
python3 scripts/benchmark-real-audio.py run \
  --output /tmp/real-audio-v1-candidate.json
python3 scripts/benchmark-real-audio.py compare \
  --baseline stratum-dsp/benchmarks/real-audio-v1/baseline.json \
  --candidate /tmp/real-audio-v1-candidate.json
```

The runner analyses the first 90 seconds using the production `analyze_audio`
pipeline and the exported Rekordbox PQTZ beat grid. Audio and grid SHA-256
fingerprints prevent a comparison from silently using different inputs.
Expected ranges are expressed as bounded deltas from the checked-in baseline:
1 BPM, 0.25 stabs/second, 0.15 template-score points, and two circular histogram
bins. Template names, rate bases, and kick-pattern labels must match exactly;
presence and fingerprint changes fail independently. Processing time remains
informational because it is hardware- and load-dependent.

Baseline updates should be reviewed like fixture changes: inspect the tabular
diff, explain intentional signal changes, and listen to affected passages
before replacing `baseline.json`. Do not normalize a regression by blindly
refreshing the baseline.

## V2 scope

Track benchmark v2 as a GitHub issue when it becomes active work. The issue
should add a versioned snapshot of the growing `genre_verified` playlist,
stratified positive and negative genre controls, time-ranged listening notes,
distribution-level feature checks, classifier metrics, and explicit
performance/memory envelopes. It should also decide whether a small,
redistributable audio subset can run in CI. V1 deliberately stays frozen so
that growing the playlist does not move the regression target underneath DSP
development.
