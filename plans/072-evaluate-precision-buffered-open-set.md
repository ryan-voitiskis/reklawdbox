# Plan 072: Evaluate precision-buffered open-set calibration

> **Status:** In progress; protocol frozen before candidate implementation
> **Objective:** Test whether a fixed five-point inner-calibration precision
> buffer makes the strongest Plan 071 formulation safe enough for the already
> sealed independent holdout without sacrificing useful coverage.

## Evidence and motivation

Plan 071 is a valid bounded development negative. Its independent-binary O2
candidate offered 297 of 716 rows at 91.58% exact precision and passed every
preregistered gate except the explicit non-target false-offer cap. It falsely
offered 16 of 117 non-target rows, or 13.68%, against a maximum of 10%. Every
outer fold offered at least 47 rows at 86.67% precision or better, five output
parents had at least eight offers, and paired precision improved on v0.33 by
15.51 percentage points.

Aggregate development diagnostics show that ten of the sixteen non-target
false offers were House suggestions. The errors were not concentrated in one
audio file or one identity and no sealed-holdout evidence was inspected. The
thresholds had been selected at exactly the same 90% precision floor later
required by the gate, leaving no calibration-to-evaluation generalization
buffer.

This plan tests one conservative response: raise only the inner calibration
target from 90% to 95%. It does not alter the model, features, labels, folds,
ridge penalty, collision rule, output parents, development gates, or release
gate. The five-point buffer is fixed before evaluation and is not searched.

## Frozen inputs

Reuse the replayed Plan 071 development evidence exactly:

- development manifest SHA-256:
  `dfd11addd96a2e7b5727700594b337aaacfc19bdd97db408e1ba0955f80853bd`;
- label-blind feature manifest SHA-256:
  `6bf80b80f060649877a90a5d6dfa8188c9549eaa0986f1667d611e115689b682`;
- cache-native feature artifact SHA-256:
  `f3e615a89f5b3770e170f0b7ddafd29e87052fcbf6a44c333ba0f9aced331365`;
- cache-native summary SHA-256:
  `87fbdb446ca18dd251f4596cbd5879d999b33d0e3463b257b1fe25ee9f043c16`;
- CLAP feature artifact SHA-256:
  `72fbace49fdcb2885d4dce78fac3f1212baac1742718d903c6203314f4e4ffc9`;
- CLAP summary SHA-256:
  `764da176061e9087d3d5d5498b17cb24fd1897aaa4ee163b5901234bca2de41b`;
- Plan 071 evaluator source SHA-256:
  `8f5d97c80fcd08e49a8062556cceec7cd48f5452ff265359446ae9ff479452d2`;
  and
- Plan 071 result SHA-256:
  `53420f41a05962341424c9754d893c70fca31ffdc9b37964ab9c07b049381e4d`.

The corpus remains 716 accepted rows in five artist- and release-isolated
folds: 599 positives across House, Ambient, Techno, Breakbeat, Reggae, Electro,
and Trance, plus 117 exact-parent non-targets. Do not append reviewed holdout
truth or modify a fold.

The fresh 60-row holdout remains identity-sealed at roster fingerprint
`81ea5361b52ac1edc5c885abb72dddbe88f352aa1b6ff599957bd444f45b1519` and
artifact SHA-256
`35968f0e3947502ede3322295b4cba6d692e6aefedc7b10940fd82ac9c43f662`.
Do not extract a holdout feature or embedding unless the sole development
candidate passes every gate.

## Sole candidate O3

Use the unchanged O2 estimator:

- seven independent class-balanced binary ridge least-squares models;
- penalty 10 and an unpenalized intercept;
- the unchanged 140 cache-native/v0.33 features;
- the unchanged locally pinned 512-value CLAP representation;
- training-partition-only PCA64 during every inner and outer fit; and
- exact canonical parent truth, with all other parents negative for every
  output.

For each outer fold, produce inner out-of-fold scores from the other four
folds. For each output parent, choose the score threshold that offers the most
inner rows while reaching at least 95% exact precision and at least eight
offers. Break ties by higher precision and then higher threshold. A parent with
no qualifying threshold is disabled in that outer fold.

On the outer fold, qualify a parent only when its binary score reaches that
parent's fixed inner threshold. Emit only when exactly one parent qualifies.
Zero qualifiers and multiple qualifiers abstain. Do not compare raw scores
between binary models.

## Development gate

Concatenate the five untouched outer-fold results once and apply the unchanged
Plan 071 gates:

- at least 180 offers and at least 25% coverage;
- at least 90% aggregate exact offered precision;
- no more than a 10% false-offer rate across the 117 non-target rows;
- every fold has at least twenty offers and at least 85% precision;
- every output with at least eight offers has at least 80% precision;
- at least four output parents have at least eight offers; and
- paired precision improves on v0.33 by at least five percentage points.

Report zero-, one-, and multi-qualified counts, per-parent and per-fold
metrics, exact non-target support and false-offer rate, and the paired v0.33
comparison. Freeze the evaluator and tests in a commit before its first live
run. Replay the result byte-for-byte.

If O3 fails any gate, record a bounded negative and stop without touching the
fresh holdout. Do not try 92%, 93%, 94%, 96%, another threshold policy, or an
additional candidate under this plan.

## Deployment calibration and independent gate

If O3 passes, calibrate its per-parent deployment thresholds from all frozen
outer out-of-fold scores using the same 95% rule. After collision abstention,
at least four parents must each retain at least eight offers at 90% exact
precision. Serialize only those thresholds and the frozen estimator contract.

Then follow the independent protocol already sealed in Plan 071:

1. audit zero path, artist, release, prior-review, and decoded-audio overlap;
2. extract and replay label-blind cache-native and CLAP holdout features;
3. fit the unchanged O3 models on all 716 development rows;
4. freeze all 60 predictions before any listening;
5. export only offers in prediction-blind batches of at most six; and
6. freeze every human verdict before the prediction join.

The independent release gate remains at least 30 of 60 offers, at least 90%
aggregate exact-primary-parent precision, at least 80% precision for every
emitted parent with five or more offers, and passing isolation and replay
audits. Product implementation remains unauthorized until that gate passes.
