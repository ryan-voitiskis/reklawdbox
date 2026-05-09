# Chord-Stab Template Investigation

Empirical study of how the Stage 2 beat-relative offset histogram behaves on
real Dub Techno tracks, and what it tells us about the templates Stage 3
needs to match against.

## Corpus

24 tracks from the `genre_verified` Rekordbox playlist tagged `Dub Techno`.
Tagged BPM range 115–130. Mix of 16-, 24-bit WAV and FLAC sources.

Each track was analysed for the first 90 s after decode, with frame=2048,
hop=512.

## Headline finding: the detector is critically grid-dependent

We ran the corpus twice — once with `generate_beat_grid` (HMM Viterbi seeded
with the tagged BPM), once with the user's hand-verified Rekordbox grid
loaded from the `PQTZ` tag of the per-track `ANLZ0000.DAT` file.

| Metric | HMM grid | Rekordbox grid |
|---|---|---|
| Tracks where global peak landed in 1/4-, 1/2-, or 3/4-off region | 14 / 24 (58%) | **20 / 24 (83%)** |
| Tracks where peak shifted >2 bins between runs | — | 15 / 24 |
| Mean off-beat concentration (off / total) | 11 % | 15 % |
| Bar count for `Excentric` (90 s) | 1 | 44 |
| Bar count for `00946.wav` (90 s) | 1 | 46 |

Two discoveries fall out of this:

1. **`generate_beat_grid` is unreliable on real audio.** It emits duplicate
   beats (caught by the new `validate_beat_grid` validator added in PR 2),
   and bar detection collapses to a single bar across long windows on
   roughly a third of the corpus. Both are upstream bugs; both should be
   filed separately.
2. **The chord-stab detector cannot be deployed without a trustworthy
   beat grid.** It is the most grid-sensitive feature in stratum-dsp by a
   wide margin — half the corpus shifts its peak by more than 2 bins
   (i.e. ≥6 % of a beat) when the grid changes. Rekordbox's hand-verified
   grids are the only currently-available trustworthy source.

## Genre structure: four canonical stab placements

With the Rekordbox grid, the dominant histogram peak across the corpus
distributes cleanly across four regions, mirroring known production
conventions:

| Template | Bin region | Tracks | Description |
|---|---|---|---|
| **Offbeat eighth (skank)** | 16 | 9 / 24 | Stab on the "&" of every beat. Basic Channel / Maurizio / Echospace / Deepchord lineage. The canonical Dub Techno chord-stab pattern. |
| **All 16th offbeats** | 8, 16, 24 | 11 / 24 (combined) | Stabs on "e", "&", and "a" of each beat. Three-peak histogram; whichever of the three dominates depends on local emphasis. Common in denser productions (Vladislav Delay, late Echospace). |
| **Anticipation / pickup** | 22–28 | 4 / 24 | A 16th-note "push" landing just before the next downbeat. Often co-occurs with the offbeat eighth. `Reminiscence` (Monolake) at bin 30 is borderline. |
| **On-beat** | 0 / 31 | 1 / 24 | Rare but real; Donato Dozzy-school minimal/deep. Boundary case: `Talis` exhibits this and may not be Dub Techno proper — it has the dub *signifiers* (delay, reverb, stab) without the *skank* placement. |

83 % of the corpus lands in the first three (off-beat) templates. A small
handful are genuine on-beat outliers, and at least one (`Talis`) is
arguably mis-tagged — boundary territory between dub techno and
deep/minimal techno that uses dub FX.

## Implications for Stage 3

1. **A single offbeat-eighth template is insufficient.** It would correctly
   match 9 / 24 tracks and would smear (not match) the 11 tracks that have
   the all-16ths pattern.

2. **A bank of three or four templates is the right shape.** Score each
   track's histogram against:
   - `offbeat_eighth`: peak at bin 16, ~zero elsewhere
   - `all_16th_offbeats`: peaks at bins 8, 16, 24, ~zero on the beat and at
     1/8-late positions
   - `anticipation`: peak at bins 22–28
   - `on_beat`: peak at bins 0 / 31 (arguably an "anti-template" — match
     here means "probably *not* Dub Techno proper")

   Take the max-scoring template and report (template, score) per track.

3. **The detector is more discriminating than the genre tag.** `Talis`
   was hand-tagged as Dub Techno but has no skank; on listening the user
   concurred it sits on a genre boundary. The detector flagging it as
   not-canonical-Dub-Techno is a feature, not a bug — but it means the
   genre_verified corpus has noisy ground truth, and any classifier built
   on top of this should not chase 100 % accuracy against the labels.

4. **Per-bar peak agreement is a useful auxiliary signal.** When the
   per-bar peak is consistent across ≥30 % of bars, the global peak is
   reliable; when it scatters, the track has either weak/absent stabs or
   a non-canonical pattern. We saw 32 / 47 bars (68 %) agreement on
   `Track 3`, which is a much stronger commitment than e.g. 13 / 47 (28 %)
   on `Cloud One`.

## Required follow-up: ANLZ parser in Rust

The chord-stab detector cannot ship as a `stratum-dsp` feature without
parsing Rekordbox's per-track beat grids from disk. The grids live in
binary `ANLZ0000.DAT` files at:

```
~/Library/Pioneer/rekordbox/share/PIONEER/USBANLZ/<hex>/<uuid>/ANLZ0000.DAT
```

The relevant tag is `PQTZ` (legacy CDJ format, present in every track):

- Tag header: 12-byte envelope (`PQTZ` magic + length).
- Body: array of 8-byte beat entries. Each entry: `(beat_number: u8, _pad: u8, bpm_x100: u16, time_ms: u32)`, big-endian. `beat_number == 1` is a downbeat.

The extended format `PQT2` (in `ANLZ0000.EXT`) carries a sparse anchor list
(tempo segments) instead of dense beats; it can be ignored for our use case
since `PQTZ` is always present and dense.

Implementation outline:

1. Add a small `anlz` module to `stratum-dsp` (or a sibling crate) that:
   - Opens an ANLZ file, walks tag envelopes, returns the `PQTZ` body.
   - Decodes the beat array into `Vec<BeatEntry { beat_number, bpm, time_s }>`.
2. Add a public function `read_beat_grid_from_anlz(path) -> Result<BeatGrid>`
   that converts the entry list into the `BeatGrid` struct dub_stab
   already accepts (`beats`, `bars` = downbeats, `downbeats`).
3. In `reklawdbox`, expose this via the existing `analyze_track_audio`
   tool by joining `djmdContent.AnalysisDataPath` with the user's
   USBANLZ root.

Reference: pyrekordbox (`pyrekordbox.anlz.AnlzFile`) and Deep Symmetry's
ANLZ documentation at <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>.

## Validation tooling (this session)

- `stratum-dsp/examples/dub_stab_real_audio.rs` — single-track Stage 1+2
  driver. Optional third arg loads grids from a JSON file produced by:
- `scripts/dump_rekordbox_grids.py` — Python helper using `pyrekordbox` to
  walk a list of track IDs, read each track's `PQTZ`, and emit
  `{file_path: {beats: [...], bars: [...]}}` JSON.
- `stratum-dsp/examples/run_dub_stab_batch.sh` — runs the example over all
  24 corpus tracks. Set `GRID_JSON=<path>` to use Rekordbox grids.

Once the Rust ANLZ parser exists, both the JSON-loading branch and the
Python helper can be deleted.
