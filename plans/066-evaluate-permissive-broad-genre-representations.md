# Plan 066: Evaluate permissively licensed broad-genre representations

> **Status:** Development inputs frozen; sealed holdout unopened
> **Objective:** Determine whether one production-plausible audio representation
> can turn the useful Plan 065 kick evidence into release-grade, selective broad
> genre suggestions.

## Why this plan exists

Plan 065 reached 90.61% offered precision at 54.03% coverage, but failed the
frozen fold-stability and supported-target guards. That result established two
things: broad genre is a useful product target, and beat-relative kick evidence
adds independent signal. It did not authorize another adjustment to the same
Discogs-EffNet adapter.

This plan therefore changes the representation, not the labels, folds, ridge,
confidence calibration, or release gate. It also seals a future 60-track
holdout before either new representation is inferred. The holdout may only be
opened if a preregistered development candidate passes every gate.

## Representation and licence audit

The audit considered models that can produce audio embeddings locally and have
a plausible path to an optional managed runtime. Commercial use and
redistribution must not be prohibited.

| Representation                       | Decision            | Reason                                                                                                            |
| ------------------------------------ | ------------------- | ----------------------------------------------------------------------------------------------------------------- |
| OpenL3 music, mel128, 512 dimensions | Evaluate            | MIT code, CC BY 4.0 weights, 18.7 MB ONNX model, and a small local inference path                                 |
| LAION CLAP HTSAT unfused             | Evaluate            | Apache-2.0 model card, audio-native 512-dimensional embeddings, and a pinned 614.5 MB checkpoint                  |
| MAEST                                | Reject              | MTG publishes its models under CC BY-NC-SA 4.0 unless separately licensed                                         |
| MERT v1 95M                          | Reject              | CC BY-NC 4.0 weights                                                                                              |
| MuQ                                  | Reject              | CC BY-NC 4.0 weights                                                                                              |
| Microsoft BEATs                      | Reject for this run | MIT project, but the official AS2M checkpoint link was not anonymously retrievable during the audit               |
| PANNs Cnn14                          | Defer               | Permissive and retrievable, but general AudioSet transfer is less music-specific than the two selected candidates |

Pinned sources:

- OpenL3 source revision
  `b0bf9b627ed943414324af2ba350917d764207c3`; upstream weights are
  [CC BY 4.0](https://github.com/marl/openl3#model-weights-license).
- OpenL3 model:
  `https://essentia.upf.edu/models/feature-extractors/openl3/openl3-music-mel128-emb512-3.onnx`.
- CLAP model revision
  `8fa0f1c6d0433df6e97c127f64b2a1d6c0dcda8a`; the pinned
  [model card](https://huggingface.co/laion/clap-htsat-unfused/tree/8fa0f1c6d0433df6e97c127f64b2a1d6c0dcda8a)
  declares Apache-2.0.

An evaluation pass is not a distribution decision. A separate implementation
plan must preserve attribution, inventory runtime dependencies, confirm every
redistributed byte's licence, and measure installation size and inference cost.

## Evidence boundary

### Development corpus

Use the current 668-row private Plan 059 manifest:

- manifest SHA-256:
  `1e877734477c25dcd622837bea8c2a0d1ae84f44d4526ef71d4645aa3fe54c3f`;
- corpus fingerprint:
  `sha256:b88911c7b24bbeecd1d59607ceb5e873ca29ff6f15052e77913635e5832471f1`;
- fold count: five; and
- truth, fold, arrangement, baseline recommendation, row order, and broad
  taxonomy remain unchanged.

The older 670-row development manifest remains an exclusion input for the
holdout so that removing two unavailable audio files cannot make their artists
or releases eligible.

Reuse these frozen inputs after validating their row identity and order against
the 668-row manifest:

- Discogs-EffNet style scores and 1280-dimensional embeddings from Plan 059;
- the Plan 065 74-value kick vector, restricted by exact path to the 668 rows;
- the v0.33 broad recommendation one-hot; and
- the four arrangement descriptors.

Private identities, features, predictions, scores, thresholds, and margins
stay outside Git.

### Sealed holdout

Run `scripts/research/select_broad_genre_holdout.py` before new representation
inference. It must:

1. read `master.db` through SQLCipher's read-only mode;
2. start from the already-unexposed Plan 060 candidate manifest;
3. exclude every exact path and every normalized artist present in either
   development manifest, any Plan 060 exclusion playlist, `genre_verified`, or
   either completed six-track review;
4. exclude missing files, blank artists, `Experimental`, and unmapped current
   genres;
5. canonicalize only known compatibility aliases before using current genre as
   a sampling stratum, never as truth;
6. select exactly 60 rows by deterministic round-robin across available broad
   strata, with at most five rows per stratum and at most one row per normalized
   artist or artist-release group; and
7. write identities only to a mode-0600 private artifact and expose only input
   hashes, counts, target distribution, and the roster SHA-256.

The seed is `broad-genre-next-model-holdout-v1`. Selection occurs before any
OpenL3 or CLAP output exists. The roster cannot be replaced or supplemented
after development results are known.

Recorded seal:

- private artifact SHA-256:
  `532ce77378154949f2f02e5283c9c12ec528639f3be912aa2ddba2ba71b35589`;
- private roster SHA-256:
  `e90b400645d89b287aab4300465fd0893314830bc6ec8b6ab22b5f9de4fbfdf9`;
- library snapshot SHA-256:
  `553dcfd0c5526f2ca8309f1a53097ac58749c8d54216879c80390c355264e653`;
- 60 rows, 60 normalized artists, and 60 artist-release groups;
- 707 eligible rows across 398 artists and 479 release groups; and
- broad sampling strata: Ambient 4, Breakbeat 4, Disco 4, Downtempo 4,
  Drum & Bass 4, Electro 5, Garage 1, Hardcore 1, House 5, IDM 2, Minimal 2,
  Pop 5, R&B 3, Reggae 3, Tech House 3, Techno 5, and Trance 5.

The selector replayed byte-identically before representation extraction. These
strata describe only how the blind roster was sampled; they are not holdout
truth and will not be shown during listening review.

## Frozen representation extraction

All audio is read only. Do not write audio, tags, Rekordbox data, caches, staged
changes, or XML.

### OpenL3

- model: `openl3-music-mel128-emb512-3.onnx`;
- sample rate: 48 kHz mono;
- input: 128 Slaney mel bands using the official OpenL3/Essentia preprocessing;
- excerpts: twelve deterministic one-second excerpts evenly spaced across the
  full decoded duration, padding only tracks shorter than one second;
- patch output: 512 values; and
- track output: patch L2 normalization, arithmetic mean, then track L2
  normalization.

### CLAP

- model: `laion/clap-htsat-unfused` at the pinned revision above;
- sample rate: 48 kHz mono;
- excerpts: three deterministic ten-second excerpts evenly spaced across the
  full decoded duration, using repeat-padding only when a track is shorter than
  ten seconds;
- explicitly avoid random truncation by supplying exact-length excerpts;
- patch output: `ClapModel.get_audio_features`, 512 values; and
- track output: patch L2 normalization, arithmetic mean, then track L2
  normalization.

Extraction is label-blind. Freeze model, extractor-source, ordered-source, and
feature-artifact SHA-256 values in this plan before classifier evaluation.

Frozen model and extractor record, committed before development audio
extraction:

- OpenL3 ONNX SHA-256:
  `81c24c8a723054717fdea5c7448acb6023baaf70a0fc526deb030c2032db0ed3`
  (18,740,670 bytes);
- CLAP weights SHA-256:
  `1cd3c601bc4afe0fa87be3de4c13dd2cfadd249fac1e29acf74a9b296c3219bb`
  (614,525,833 bytes);
- CLAP config SHA-256:
  `9efb9557bc804f2ca6e394486af2e45dfed0b18554909735a99c6220b84e4288`;
- CLAP preprocessing config SHA-256:
  `9739f58296aa6f9ac18008fd0150fb2649bc554985fbde86d0a4041c882ac753`;
- extractor source SHA-256:
  `5d09431f18e77320a8f0c77b0af393cce4ea8979275cfb6adc2b7b5f44fd7e5c`;
- OpenL3 evaluation runtime: NumPy 2.5.1, Essentia 2.1-beta6-dev,
  ONNX Runtime 1.28.0; and
- CLAP evaluation runtime: NumPy 2.3.5, PyTorch 2.13.0, Transformers
  4.57.6, MPS float32 inference.

Both extractors passed a synthetic-audio end-to-end check with their frozen
patch count, a `(patches, 512)` model output, a finite 512-value track output,
and unit L2 norm. This validated mechanics only; no development or holdout
classification was inspected.

The completed label-blind development extraction is frozen as follows:

- ordered decoded-source SHA-256 for both representations:
  `b4c9a9df9516bd0819ac8f3687f9087814d7bc5995d2ca7bb00c7fc28484d2c4`;
- OpenL3 feature artifact SHA-256:
  `d9c06b2df65199d98e17277a268e69732e41c7e7b76d6f9e2c82824461b8097c`;
- OpenL3 extraction summary SHA-256:
  `82951bbc023d49cea1c1ade10d7808da95778397fc871063646e043e65393b09`;
- CLAP feature artifact SHA-256:
  `097443ac6ec6f0195ce8904643ec74703b3a81c50ed9d0610213b7674970d59a`;
- CLAP extraction summary SHA-256:
  `ccaf9fbbda54c086faf2b1856b27cf4194e830998635a30e292eb830eb2da745`;
  and
- each artifact contains 668 finite, unit-normalized, 512-dimensional rows.

The evaluation implementation is also frozen before its first
genre-conditioned run:

- evaluator source SHA-256:
  `384c0d62993fcbefb689b97c884935ee94579c01175f39e2472e50a91ee78bf5`;
- broad evaluator support source SHA-256:
  `25bc75ccca3c2122be1ec3037054e8f934b8cd4438b67d9a0166a34d77661558`;
- supervised evaluator support source SHA-256:
  `ade670e4b25689e0f3175829300744715e8d15f2803ddf5e63c41f092e2d8ce2`;
- kick evaluator support source SHA-256:
  `da2ad49e9797a75c4f5573c5be9e9138708306b218ab59480ab74172df6c5746`;
  and
- 16 focused evaluator and inherited adapter tests pass.

## Frozen candidates

Evaluate exactly two candidates, one per new representation. Each appends a
training-partition PCA64 projection of that representation to the unchanged
Plan 065 feature vector:

1. broad maximum Discogs-EffNet style scores;
2. mapped v0.33 broad one-hot;
3. four arrangement descriptors;
4. training-partition PCA64 of the original 1280-dimensional EffNet embedding;
5. the frozen 74-value kick vector; and
6. training-partition PCA64 of either OpenL3 or CLAP.

Use the Plan 065 class-balanced one-versus-rest ridge head with penalty 10.0,
unpenalized intercept, five outer folds, nested inner out-of-fold margin
calibration, minimum-offer rule, threshold tie-break, abstention behavior, and
deterministic broad-target order. All imputation, standardization, and both PCA
fits occur inside the active training partition.

There is no embedding concatenation between OpenL3 and CLAP, no fine-tuning,
zero-shot text prompt, target-specific threshold, class prior, BPM rule, or
third representation.

### Frozen full-fit holdout model

The nested fold-local thresholds are a development stability test, not a
deployable threshold. For each candidate, use its five outer out-of-fold
predictions and margins to select one global threshold with the unchanged 90%
precision target, maximum-offer rule, and deterministic tie-break. Require at
least `max(60, ceil(10% of development rows))` offers during calibration.

Apply that one threshold to the same outer out-of-fold margins and require the
entire development gate below a second time. A candidate is holdout-ready only
if both its nested fold-local gate and this global deployment-threshold gate
pass.

For a holdout-ready candidate, fit both PCA transforms, imputation,
standardization, and the ridge head once on all 668 development rows. Apply the
global out-of-fold threshold unchanged to final-model holdout margins. The
holdout inputs must be exact Plan 060 rows joined by path and must use the
already-frozen Plan 065 kick extractor, Discogs-EffNet inputs, baseline broad
one-hot, arrangement descriptors, and only the selected new representation.
Do not use current genre or sampling stratum as an input.

If both candidates pass, select by this fixed order:

1. higher minimum outer-fold offered precision under the global deployment
   threshold;
2. higher minimum supported-target offered precision under that threshold;
3. higher overall offered precision under that threshold;
4. higher coverage under that threshold; then
5. OpenL3, because its runtime is materially smaller.

## Frozen development gate

A candidate advances only if every Plan 065 release-development check passes
for both nested fold-local selection and the single global deployment
threshold:

1. offered precision is at least 0.90;
2. coverage is at least 0.50;
3. every outer fold makes an offer and has precision of at least 0.85;
4. every target with at least ten truth rows and at least five offers has
   precision of at least 0.75;
5. selective precision improves on that candidate's unselective precision by
   at least 0.10; and
6. every input hash, semantic checksum, row order, fold, and truth index
   matches.

Plan 065 deltas are diagnostics only. No gate changes after results.

## Holdout and release boundaries

If neither candidate passes, record a bounded negative and do not open the
holdout. If one passes:

1. freeze that candidate's complete model, preprocessing, head, thresholds,
   and semantic checksum;
2. infer the sealed roster once and seal predictions before listening;
3. present blind listening batches of four to six without current genre,
   prediction, confidence, artist context beyond ordinary track identity, or
   model rationale; and
4. accept `broad genre`, `ambiguous`, or `skip` as valid operator answers.

The holdout passes only if at least 30 of 60 rows receive offers, offered
precision is at least 0.90, and no broad target with at least five reviewed
offers falls below 0.75 precision. Ambiguous and skipped rows are not counted
correct or incorrect but remain in the denominator for offer coverage.

A pass authorizes a separate experimental product implementation, not an
automatic metadata writer. The first surface must be read-only by default,
show broad and fine suggestions separately, abstain explicitly, and stage a
broad genre only after an operator request through `ChangeManager` and
`write_xml`.

## Verification

- Unit-test alias projection, holdout exclusions, deterministic selection,
  artist/release isolation, target cap, exact roster size, extraction shapes,
  patch aggregation, fold-local PCA, and two-embedding isolation.
- Require byte-identical holdout selection and aggregate result replay.
- Inspect commits for private identities, paths, features, predictions, scores,
  and margins.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

This plan is complete when the holdout is sealed before inference, both fixed
representations have either run or failed a hard licence/runtime prerequisite,
the frozen development result replays byte-identically, and either a passing
candidate is ready for blind holdout review or the bounded negative is recorded.
