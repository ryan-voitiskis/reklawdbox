# Genre Reference Candidate Corpus

**Research snapshot:** 2026-07-18

**Status:** Seven reusable dossiers complete; additional research is gap-driven

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

When a measured classifier gap justifies a new or refreshed dossier, that
dossier needs at least three definition sources from at least two independent
publishers, including an institutional, academic, archival, first-party
artist/label, established historical, or respected scene source. Every
candidate needs two independent canonicality sources, separate version/release
metadata, and at least one opened direct digital-purchase page. Access dates
describe only this snapshot; availability must be rechecked before any future
purchase decision.

A completed dossier uses at least 12 candidates: four foundational, four
representative, two contemporary, and two boundary recordings. The set spans
at least eight lead acts, four labels, and three meaningful eras or scene
generations. An act appears at most twice and one label supplies no more than
25 percent of a genre's set. Empty dossiers are a coverage map, not mandatory
backlog.

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
other cross-genre uses of dub production techniques. The candidate corpus
retains records for 52 possible targets: 53 live canonical names minus
`Experimental`.

The 2026-07-18 pilot checkpoint populates 7 of those 52 dossiers with 84
source-verified candidates. Every populated genre meets the complete
per-genre evidence, diversity, availability, role, and leakage requirements.
The remaining 45 dossiers are intentionally empty. They are populated only
when broad evaluation identifies a concrete genre or confusion boundary that
needs more evidence.

## Taxonomy Decision: Tech House Is Multi-Strain

The Tech House label does not identify one stable sound across its history.
The revised dossier distinguishes these overlapping strains:

- the early London DJ practice, including Wiggle's raw, stripped and dubby
  direction and Heart & Soul's Detroit-and-breaks direction;
- late-1990s UK production shaped by House shuffle, dub bass, Chicago jack,
  Detroit musicality, and West Coast jacking or tribal percussion;
- the cleaner European minimal-polished and electro-house-influenced 2000s
  form;
- 2010s Ibiza vocal anthems and percussion-led large-room tools;
- contemporary overlap with the minimal/deep-tech umbrella, whose artists,
  labels, venues, and audiences intersect with Tech House even when individual
  recordings remain better classified as House, Deep House, or Minimal; and
- an EDM- and Electro House-tinged commercial festival branch heard
  internationally and in Australian commercial-leaning events.

The last branch is legitimate scene and retail usage, but it is too sonically
divergent to define the pilot's positive center without separate listening and
benchmark evidence. On 2026-07-18 the operator removed FISHER's `Losing It`
and `You Little Beauty` from the candidate set as insufficiently aligned
positive references. East End Dubs' `Dis` replaced one, representing the
rolling, bass-led London underground.

On 2026-07-19 listening review also rejected Chris Stussy's `Desire`. Its
appearance in a current minimal-tech-house feature established shared scene
context, but the recording sounded like House and the artist's official tags
were House, Deep House, and Minimal. The corpus replaced it with Mr. G's `Home
Alone (on my b'day)`, whose rugged jacking drums, bass weight, restrained vocal,
and hypnotic development more directly continue the underground Tech House
target.

This is a curation decision, not a claim that commercial festival Tech House
does not exist. That branch remains a documented review cohort. A later
leakage-controlled benchmark must determine whether one classifier target can
cover it alongside the underground strains without reducing accuracy for both.

## Coverage Matrix

This matrix summarizes the JSON; it is not a second track list. Counts and
statuses must be reconciled whenever the structured corpus changes.

| Genre | Family | Disposition | Candidates | Status / caveat |
|---|---|---:|---:|---|
| 2-Step Garage | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Acid | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Afro House | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Ambient | Downtempo | audio_reference | 0 | Unpopulated; gap-driven only |
| Ambient Techno | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Bassline | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Breakbeat | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Broken Beat | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Dancehall | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Deep House | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Deep Techno | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Disco | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Downtempo | Downtempo | audio_reference | 0 | Unpopulated; gap-driven only |
| Drum & Bass | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Dub | Downtempo | audio_reference | 12 | Pilot complete; Dub Reggae and Reggae Dub normalize here; Reggae and Dub Techno boundaries retained |
| Dub Techno | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Dubstep | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| EBM | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Electro | Techno | taxonomy_review | 12 | Pilot complete; machine-funk lineage retained; Electro House excluded |
| Footwork | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Future Garage | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Gabber | Hardcore | audio_reference | 0 | Unpopulated; gap-driven only |
| Garage | House | taxonomy_review | 12 | Pilot complete; US Garage target bounded against UK Garage descendants |
| Gospel House | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Grime | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Happy Hardcore | Hardcore | audio_reference | 0 | Unpopulated; gap-driven only |
| Hard Techno | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Hard Trance | Hardcore | audio_reference | 0 | Unpopulated; gap-driven only |
| Hardcore | Hardcore | taxonomy_review | 12 | Pilot complete; broad electronic-Hardcore target bounded against Gabber, Hardstyle, and Jungle |
| Hardstyle | Hardcore | audio_reference | 0 | Unpopulated; gap-driven only |
| Highlife | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Hip Hop | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| House | House | audio_reference | 0 | Unpopulated; gap-driven only |
| IDM | Downtempo | taxonomy_review | 12 | Pilot complete; contested umbrella retained with explicit Ambient, Breakbeat, and Techno boundaries |
| Italo Disco | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Italodance | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Jazz | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Jungle | Bass | audio_reference | 0 | Unpopulated; gap-driven only |
| Minimal | Techno | taxonomy_review | 12 | Pilot complete; minimal electronic target bounded by scene and arrangement, not sparsity alone |
| Pop | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Progressive House | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Psytrance | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| R&B | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Reggae | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Rock | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Speed Garage | House | audio_reference | 0 | Unpopulated; gap-driven only |
| Synth-pop | Other | metadata_led | 0 | Unpopulated; gap-driven only |
| Tech House | House | taxonomy_review | 12 | Pilot complete; multi-strain target spans London roots, minimal-polished and Ibiza forms, plus underground continuity; minimal/deep-tech adjacency and the commercial festival branch remain review cohorts |
| Techno | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Trance | Techno | audio_reference | 0 | Unpopulated; gap-driven only |
| Trip-Hop | Downtempo | audio_reference | 0 | Unpopulated; gap-driven only |
| UK Funky | House | audio_reference | 0 | Unpopulated; gap-driven only |

## Research Conclusions And Limitations

The live taxonomy contains 53 names. The corpus retains the 52 records left
after excluding only `Experimental`, but there is no longer a requirement to
populate all of them. The pilot completed bounded definitions and candidate
sets for `Minimal`, `Garage`, `Electro`, `Hardcore`, `IDM`, `Dub`, and
`Tech House`. Classifier families are organizational metadata, not
musicological truth. If a later evaluation identifies an ambiguous label such
as `Acid`, `Trance`, or `Downtempo` as a measured limitation, its gap-driven
dossier must use the same explicit definition and exclusion standard.

Broad controls such as Pop, Rock, Jazz, R&B, and Highlife test electronic
library boundaries; their candidate sets do not claim to exhaustively define a
world genre. Medium-confidence and taxonomy-review cases require especially
careful listening. Legal availability, source pages, and retailer catalogs can
change after the recorded access date.
