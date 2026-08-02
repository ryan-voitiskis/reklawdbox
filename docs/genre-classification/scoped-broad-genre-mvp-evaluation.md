# Scoped Broad-Genre MVP Evaluation

**Run date:** 2026-08-02

**Status:** Independent holdout coverage failed; no listening review or release

## Question and boundary

Plan 067 asked whether an explicit Ambient, House, and Techno suggestion scope
could deliver useful broad classification before the full 26-target truth
corpus is ready. Predictions outside those roots were forced to abstain.

The scope was derived after Plan 066 development results, so a new independent
48-track holdout was sealed before scoped fold metrics were computed. It
excluded development artists and releases, prior review rows, and the unopened
Plan 066 holdout. Current genre was sampling metadata only and was absent from
the model and representation manifests.

## Development result

Both fixed candidates passed every preregistered scoped development gate.

| Candidate | View | Offers | Coverage | Offered precision | Worst fold precision |
|---|---|---:|---:|---:|---:|
| OpenL3 | Nested | 276 | 41.32% | 94.57% | 90.38% |
| OpenL3 | Global deployment threshold | 270 | 40.42% | 95.56% | 93.75% |
| CLAP | Nested | 297 | 44.46% | 95.96% | 94.55% |
| CLAP | Global deployment threshold | 320 | 47.90% | 95.00% | 93.10% |

OpenL3 was selected by the frozen runtime priority because both candidates
passed. Its ONNX model is 18.7 MB, compared with CLAP's 614.5 MB checkpoint.
The development result replayed byte-identically.

## Independent holdout

The full-fit OpenL3 model used all 668 development rows and the unchanged
global threshold `0.25702530873209417`. All inputs and the inference source were
committed before the first holdout score.

| Holdout measure | Required | Observed | Result |
|---|---:|---:|---|
| Model offers | At least 20 of 48 | 14 of 48 | Fail |
| Offer coverage | Diagnostic | 29.17% | — |
| Offered predictions by root | Diagnostic | Ambient 4; House 6; Techno 4 | — |

The offer-count condition is independent of human truth. Once it failed,
listening could not make the candidate pass. Predictions and confidence were
therefore not shown to the operator, and offered precision was not measured.
The prediction artifact replayed byte-identically.

## Decision

The scoped MVP is a bounded negative. It is not a production feature and does
not authorize a model dependency or public suggestion surface. The failure is
coverage, not demonstrated inaccuracy: development precision was strong, but
the independent roster received too few confident in-scope offers to satisfy
the utility floor.

Do not lower the threshold, enlarge the allowlist, run CLAP on the same
holdout, or inspect the 14 predictions as a rescue attempt. The original Plan
066 holdout remains sealed.

The next justified investment is truth, not another exposed-corpus threshold
search. The current corpus has no examples for eight broad targets and only
10–20 for several unstable roots. The retired Plan 067 roster may now be
repurposed as fresh training truth in blind batches of at most six, provided it
is never again described as holdout evidence.

## Reproducibility

- scoped development result SHA-256:
  `baf0045315fd48ad19be92f209402e75d0af84815aa6a90bb8bf7b637ceaeea9`
- holdout roster artifact SHA-256:
  `7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f`
- holdout OpenL3 feature SHA-256:
  `45dcd030c73bd236dd8aab772ab034af0613f8bc537f1720f4f18b1408c7efe9`
- full-fit inference source SHA-256:
  `6a82536204978af1d10dfc609cbf8de5b751b7b4d4ab20fb59928b840b1efd96`
- private prediction artifact SHA-256:
  `6e52c5b85397a94f4e25174844a11f10fda4cbf6f60141dcf53eebd73dbdd6c6`
- result replay: byte-identical
- listening rows exposed: zero
