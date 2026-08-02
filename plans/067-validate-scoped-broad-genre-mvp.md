# Plan 067: Validate a scoped broad-genre MVP

> **Status:** Preregistered; scoped holdout not yet sealed
> **Objective:** Determine whether a deliberately limited set of broad-root
> suggestions can deliver useful, honest value before the full 26-target truth
> corpus is ready.

## Why this plan exists

Plan 066 was a productive bounded negative. CLAP reached 91.56% nested offered
precision at 60.33% coverage and cleared every fold-stability check, but severe
errors in sparse roots prevented an all-target release. OpenL3 was weaker
overall but uses an 18.7 MB model rather than CLAP's 614.5 MB checkpoint.

The development corpus contains truth for only 18 of the frozen 26 broad
targets. Eight have no training examples: Acid, EBM, Footwork, Grime, Jazz,
Pop, R&B, and Rock. Repeating model or threshold searches cannot repair that
coverage gap.

The strongest near-term product path is therefore a scoped selective MVP, not
an implied 26-target classifier. It may suggest only roots that already have
enough development truth and strong Plan 066 precision; every other prediction
must abstain. This rule is derived after seeing Plan 066 development results,
so it requires a new independent holdout and cannot use the Plan 066 holdout.

## Frozen product scope

The initial six-root allowlist was the Plan 066 CLAP targets with at least 20
truth rows and at least 90% nested offered precision. Holdout preflight then
showed that, after all artist-level exclusions, only three of those roots have
at least eight independent current-tagged candidate rows. The release-testable
allowlist is therefore:

- Ambient;
- House;
- Techno.

The rule is frozen as `scoped-broad-roots-v1`. A candidate prediction outside
this set is an abstention regardless of confidence. The product must disclose
this scope and must not describe abstention as an `Other` genre.

This is intentionally post-development product scoping. The independent
holdout, not the exposed Plan 066 rows, is the release test.

## Frozen candidates

Evaluate exactly two candidates by replaying Plan 066 without retraining,
changing a feature, or changing a threshold:

1. **OpenL3 scoped:** Plan 066 OpenL3 outer predictions, nested fold-local
   thresholds, and global deployment threshold
   `0.25702530873209417`, followed by the allowlist.
2. **CLAP scoped:** Plan 066 CLAP outer predictions, nested fold-local
   thresholds, and global deployment threshold
   `0.18173795988087316`, followed by the allowlist.

All Plan 066 base features, PCA64 transforms, ridge penalty 10.0,
class-balancing, folds, preprocessing, target order, and hashes remain
unchanged. Do not combine representations or add target-specific thresholds.

OpenL3 is selected if it passes both development views because its model is
about 33 times smaller. CLAP is selected only if OpenL3 fails and CLAP passes.
This product-runtime preference is frozen before scoped fold metrics are
computed.

## Frozen development gate

Both nested fold-local selection and the global deployment threshold must pass:

1. offered precision at least 0.90;
2. coverage at least 0.40;
3. every fold makes an offer and has precision at least 0.85;
4. every allowlisted target with at least five offers has precision at least
   0.85;
5. selective precision improves on the same candidate with only the allowlist
   filter and no margin threshold by at least 0.10; and
6. every Plan 066 source, artifact, row, fold, truth, and semantic checksum
   matches.

The lower coverage floor reflects the explicit three-root product scope. It is
still materially above v0.33's 30.54% High/Medium broad coverage. No gate
changes after scoped metrics are visible.

## New sealed holdout

Before implementing the scoped evaluator, select a new 48-track roster with
seed `scoped-broad-genre-mvp-holdout-v1` from the unexposed Plan 060 audit
universe. The selector must:

1. read `master.db` through SQLCipher read-only mode;
2. exclude every path, normalized artist, and artist-release group in either
   Plan 059 development manifest, every prior listening exclusion, the Plan
   066 holdout, or `genre_verified`;
3. exclude missing files, blank artists, `Experimental`, and unmapped current
   genres;
4. use current genre only as a sampling stratum, never truth;
5. select exactly 48 rows by deterministic round-robin, with at most eight per
   broad sampling stratum and one per normalized artist and release group;
6. write identities only to a mode-0600 private artifact; and
7. expose only hashes, aggregate counts, and the roster checksum.

The Plan 066 holdout remains sealed and cannot be substituted, merged, or used
as development data. The new roster cannot change after scoped development
metrics are known.

The initially committed four-per-stratum selector stopped before writing a
roster because it could select only 31 rows. Aggregate preflight found 490
eligible rows across ten sampling strata, but only Ambient, House, and Techno
had at least eight rows among the six proposed roots; Electro had two and Hip
Hop and Reggae had none. No scoped model metric had been computed. The scope and
cap above were corrected and recommitted before selection.

## Full-fit and holdout boundary

If neither scoped candidate passes, record a bounded negative and stop. If one
passes:

1. freeze a full-fit implementation using all 668 development rows and the
   candidate's unchanged global threshold;
2. extract only the selected representation and the already-frozen base and
   kick inputs for the new holdout;
3. seal predictions, margins, offers, and artifact hashes before listening;
4. present only offered rows in blind batches of at most six, hiding current
   genre, sampling stratum, model label, confidence, and rationale; and
5. accept a broad root, `ambiguous`, or `skip` as valid review answers.

The holdout passes only if:

- the frozen model makes at least 20 offers;
- at least 18 offered rows receive a resolved broad-root answer;
- resolved offered precision is at least 0.90; and
- every predicted allowlisted root with at least five resolved offers has
  precision at least 0.80.

Ambiguous and skipped rows are not counted correct or incorrect. They remain in
the 48-row denominator when reporting model offer coverage.

## Product boundary

A holdout pass authorizes a separate implementation plan for an experimental,
read-only broad suggestion surface. It does not authorize automatic tag or
Rekordbox mutation.

The first product surface must:

- name the three supported roots;
- distinguish broad suggestions from fine genres;
- display abstention explicitly;
- never map out-of-scope predictions to a supported root after inference;
- stage metadata only after an operator request through `ChangeManager` and
  `write_xml`; and
- document model attribution, managed-runtime size, latency, and licensing.

Independent truth expansion should continue in small blind batches after this
MVP decision. Holdout answers cannot become development truth until the scoped
candidate is permanently accepted or retired.

## Verification

- Unit-test deterministic holdout selection, prior-holdout exclusion,
  artist/release isolation, target cap, and exact roster size.
- Unit-test allowlist filtering before metrics, nested replay, both gates, and
  OpenL3-first selection.
- Require byte-identical selection and aggregate-result replay.
- Inspect commits for private identities, paths, row predictions, scores, and
  margins.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

This plan is complete when the new holdout is sealed before scoped evaluation,
both fixed candidates replay once, and either one candidate is ready for blind
holdout review or the bounded negative is recorded.
