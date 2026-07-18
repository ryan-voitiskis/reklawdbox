# Genre Reference Candidate Corpus

**Research snapshot:** 2026-07-18

**Status:** Seven-genre ambiguity pilot complete; 45 genre dossiers remain

**Candidate playlist:** `genre_reference_candidates`

**Future approved training playlist:** `genre_verified`

**Future sealed holdout playlist:** `genre_reference_holdout`

## Purpose And Approval Boundary

This dossier defines a public, source-verifiable listening queue. It does not
contain audio, private Rekordbox identities, file paths, ownership status,
fingerprints, purchase history, or listening notes. A candidate is not genre
truth merely because it appears in
[the structured corpus](genre-reference-candidates.json).

The operator must listen to and approve each exact recording/version before it
can enter either future truth pool. Approval for training does not approve a
related edit, remaster, remix, or compilation appearance for holdout. No
classifier profiles should be recalibrated until a separate benchmark plan
seals a leakage-controlled evaluation set.

`Experimental` is the sole excluded canonical entry. It is an anti-genre or
umbrella category rather than one coherent audio prototype.

## Evidence Model

The corpus keeps four questions separate:

1. **Genre fact:** what a bounded genre label means, including disputed history
   and neighboring meanings.
2. **Recording canonicality:** why the exact track or release is a foundational,
   representative, contemporary, or boundary example.
3. **Current availability:** whether an opened direct page visibly offers the
   exact recording or release as a legitimate digital purchase on the access
   date.
4. **Classifier suitability:** whether a human-approved recording is a clear
   training anchor, representative sealed-holdout candidate, or ambiguous
   boundary-review case.

A store category cannot establish canonicality. Artist reputation cannot make
every track by that artist canonical. Release metadata establishes identity,
not genre significance.

## Source And Selection Standard

Each genre needs at least three definition sources from at least two independent
publishers, including an institutional, academic, archival, first-party
artist/label, established historical, or respected scene source. Every
candidate needs two independent canonicality sources, separate version/release
metadata, and at least one opened direct digital-purchase page. Access dates
describe only this snapshot; availability must be rechecked before any future
purchase decision.

Each genre needs at least 12 candidates: four foundational, four
representative, two contemporary, and two boundary recordings. The set must
span at least eight lead acts, four labels, and three meaningful eras or scene
generations. An act may appear at most twice and one label may supply no more
than 25 percent of a genre's set.

The recommended future split is at least six training anchors, four sealed
holdout candidates, and two boundary-review recordings per genre. These are
recommendations only. Originals, remasters, edits, remixes, compilation
appearances, and other related versions share one `leakage_group` and may not
cross the recommended training/holdout boundary.

## Listening And Intake Workflow

1. Reopen the candidate's canonicality, metadata, and acquisition pages.
2. Confirm that the purchased/listened version is the documented exact mix or
   version.
3. Listen without treating the provisional target as unquestionable truth.
4. Approve, reject, or reclassify the candidate; keep private notes outside the
   repository.
5. Assign every related version in its leakage group to one future pool only.
6. Add approved training anchors to `genre_verified` or seal approved
   evaluation examples in `genre_reference_holdout` through a separately
   authorized workflow.
7. Rebuild a benchmark before any profile calibration or classifier claim.

The repository does not purchase, download, tag, analyze, import, or add these
recordings to Rekordbox as part of this research plan.

## Taxonomy Decision: Canonical Dub

The ambiguity-first pilot could not establish non-overlapping working
definitions for `Dub` and `Dub Reggae`:

- The [London Museum](https://www.londonmuseum.org.uk/collections/london-stories/dub-london-shops-sound-systems-legends/)
  explicitly presents “dub” and “dub reggae” as alternative names for the same
  reggae offshoot.
- The [Jamaica Information Service](https://jis.gov.jm/jamaica/dub/) calls Dub
  a style grown from reggae, characterized by manipulated versions, rhythm
  emphasis, echo, and reverb.
- [Wesleyan University Press](https://www.weslpress.org/9780819565723/dub/)
  describes the academic history *Dub: Soundscapes and Shattered Songs in
  Jamaican Reggae* as the history of a Jamaican-reggae subgenre.
- A [peer-reviewed history indexed by Lancaster University](https://research.lancaster-university.uk/en/publications/king-tubby-meets-the-upsetter-at-the-grass-roots-of-dub-some-thou/)
  uses “dub reggae” in its title while defining “Dub” as the same studio
  deconstruction practice.
- [Dancecult research](https://dj.dancecult.net/index.php/dancecult/article/download/678/692/2690)
  documents Dub's international expansion beyond Jamaica without naming the
  Jamaican origin as a distinct `Dub Reggae` genre.

The operator resolved the ambiguity on 2026-07-18 by retaining `Dub` as the
canonical genre and treating `Dub Reggae` and `Reggae Dub` as compatibility
aliases. This follows the strongest sourced interpretation, keeps the common
short genre name, and avoids arbitrarily dividing the foundational King Tubby,
Lee “Scratch” Perry, Errol Thompson, Augustus Pablo, and Scientist canon.

The `Dub` dossier must still exclude generic “dub mix” naming and distinguish
the Jamaican reggae-derived genre from `Reggae`, `Dub Techno`, `Dubstep`, and
other cross-genre uses of dub production techniques. The candidate corpus now
covers 52 targets: 53 live canonical names minus `Experimental`.

The 2026-07-18 pilot checkpoint populates 7 of those 52 dossiers with 84
source-verified candidates. Every populated genre meets the complete
per-genre evidence, diversity, availability, role, and leakage requirements;
the remaining 45 dossiers are intentionally empty for subsequent family
waves. This is an incomplete research checkpoint, not a completed corpus.

## Coverage Matrix

This matrix summarizes the JSON; it is not a second track list. Counts and
statuses must be reconciled whenever the structured corpus changes.

| Genre | Family | Disposition | Candidates | Status / caveat |
|---|---|---:|---:|---|
| 2-Step Garage | House | audio_reference | 0 | Research pending |
| Acid | Techno | audio_reference | 0 | Research pending |
| Afro House | House | audio_reference | 0 | Research pending |
| Ambient | Downtempo | audio_reference | 0 | Research pending |
| Ambient Techno | Techno | audio_reference | 0 | Research pending |
| Bassline | Bass | audio_reference | 0 | Research pending |
| Breakbeat | Bass | audio_reference | 0 | Research pending |
| Broken Beat | Bass | audio_reference | 0 | Research pending |
| Dancehall | Other | metadata_led | 0 | Research pending |
| Deep House | House | audio_reference | 0 | Research pending |
| Deep Techno | Techno | audio_reference | 0 | Research pending |
| Disco | House | audio_reference | 0 | Research pending |
| Downtempo | Downtempo | audio_reference | 0 | Research pending |
| Drum & Bass | Bass | audio_reference | 0 | Research pending |
| Dub | Downtempo | audio_reference | 12 | Pilot complete; Dub Reggae and Reggae Dub normalize here; Reggae and Dub Techno boundaries retained |
| Dub Techno | Techno | audio_reference | 0 | Research pending |
| Dubstep | Bass | audio_reference | 0 | Research pending |
| EBM | Techno | audio_reference | 0 | Research pending |
| Electro | Techno | taxonomy_review | 12 | Pilot complete; machine-funk lineage retained; Electro House excluded |
| Footwork | Bass | audio_reference | 0 | Research pending |
| Future Garage | Bass | audio_reference | 0 | Research pending |
| Gabber | Hardcore | audio_reference | 0 | Research pending |
| Garage | House | taxonomy_review | 12 | Pilot complete; US Garage target bounded against UK Garage descendants |
| Gospel House | House | audio_reference | 0 | Research pending |
| Grime | Bass | audio_reference | 0 | Research pending |
| Happy Hardcore | Hardcore | audio_reference | 0 | Research pending |
| Hard Techno | Techno | audio_reference | 0 | Research pending |
| Hard Trance | Hardcore | audio_reference | 0 | Research pending |
| Hardcore | Hardcore | taxonomy_review | 12 | Pilot complete; broad electronic-Hardcore target bounded against Gabber, Hardstyle, and Jungle |
| Hardstyle | Hardcore | audio_reference | 0 | Research pending |
| Highlife | Other | metadata_led | 0 | Research pending |
| Hip Hop | Other | metadata_led | 0 | Research pending |
| House | House | audio_reference | 0 | Research pending |
| IDM | Downtempo | taxonomy_review | 12 | Pilot complete; contested umbrella retained with explicit Ambient, Breakbeat, and Techno boundaries |
| Italo Disco | House | audio_reference | 0 | Research pending |
| Italodance | House | audio_reference | 0 | Research pending |
| Jazz | Other | metadata_led | 0 | Research pending |
| Jungle | Bass | audio_reference | 0 | Research pending |
| Minimal | Techno | taxonomy_review | 12 | Pilot complete; minimal electronic target bounded by scene and arrangement, not sparsity alone |
| Pop | Other | metadata_led | 0 | Research pending |
| Progressive House | House | audio_reference | 0 | Research pending |
| Psytrance | Techno | audio_reference | 0 | Research pending |
| R&B | Other | metadata_led | 0 | Research pending |
| Reggae | Other | metadata_led | 0 | Research pending |
| Rock | Other | metadata_led | 0 | Research pending |
| Speed Garage | House | audio_reference | 0 | Research pending |
| Synth-pop | Other | metadata_led | 0 | Research pending |
| Tech House | House | taxonomy_review | 12 | Pilot complete; era-aware target spans DJ-led London roots through festival scale |
| Techno | Techno | audio_reference | 0 | Research pending |
| Trance | Techno | audio_reference | 0 | Research pending |
| Trip-Hop | Downtempo | audio_reference | 0 | Research pending |
| UK Funky | House | audio_reference | 0 | Research pending |

## Research Conclusions And Limitations

The live taxonomy contains 53 names; the research scope is the exact 52 left
after excluding only `Experimental`. The pilot has completed the bounded
definitions and candidate sets for `Minimal`, `Garage`, `Electro`, `Hardcore`,
`IDM`, `Dub`, and `Tech House`. Classifier families are organizational
metadata, not musicological truth. Remaining ambiguous labels such as `Acid`,
`Trance`, and `Downtempo` still require the same explicit definition and
exclusion work in their family waves.

Broad controls such as Pop, Rock, Jazz, R&B, and Highlife test electronic
library boundaries; their candidate sets do not claim to exhaustively define a
world genre. Medium-confidence and taxonomy-review cases require especially
careful listening. Legal availability, source pages, and retailer catalogs can
change after the recorded access date.
