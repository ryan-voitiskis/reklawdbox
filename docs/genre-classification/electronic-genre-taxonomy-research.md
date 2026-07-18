# Electronic Genre Taxonomy Research

**Originally researched:** 2026-06-14
**Taxonomy audit:** 2026-07-18
**Scope:** Canonical genres in `src/domain/classification/taxonomy/catalog.rs`, with emphasis on claims that can
inform `genre_classification` rules, DSP feature design, calibration review, and
metadata weighting.

This is a working research note, not formal product documentation. The taxonomy
is intentionally not a closed list, but the classifier currently treats the
canonical genres as consistency targets and attaches family, depth, and BPM
metadata to many of them.

## Verification And Evidence Status (2026-07-18)

The direct pages in the source index were reopened on 2026-07-18. This audit
does not promote this genre-level note into track-level evidence. The structured
reference corpus must independently cite each candidate and exact release, and
must not treat a retailer category, artist reputation, or this document itself
as proof that a recording is canonical.

The audit separates four claim types used below:

- **Externally sourced genre fact:** a history, scene, instrument, rhythm, or
  production claim supported by the linked page.
- **Current classifier behavior:** a statement checked against the live
  taxonomy metadata, evidence extraction, or engine at commit `b65333d`.
- **Research hypothesis:** a proposed future feature or conjunction that is not
  current classifier behavior and requires real-audio validation.
- **Contested taxonomy judgment:** a bounded working interpretation that must
  remain visible to the listening reviewer rather than being presented as
  settled musicological truth.

Corrections and qualifications from this audit:

- **Corrected:** `Drone Techno` was removed from the canonical catalog and its
  dedicated section and code-facing recommendations were retired. It remains a
  useful descriptive phrase in external research, not a live target.
- **Corrected:** source review found no stable distinction between `Dub` and
  `Dub Reggae`. The operator retained canonical `Dub`; `Dub Reggae` and
  `Reggae Dub` now normalize to it as compatibility aliases.
- **Corrected:** `kick_pattern` is no longer a wholly missing descriptor. The
  current evidence pipeline carries the Stratum value and confidence, but the
  classification engine does not yet use it as a direct genre vote. References
  below to kick pattern are therefore future rule/template hypotheses.
- **Qualified:** the older broad guide and production-tutorial links are useful
  leads, but several individual genre sections have fewer than the three
  independent definition sources required by the reference corpus. Those
  sections must not drive candidate selection until the JSON dossier supplies
  stronger independent evidence.
- **Qualified:** Wikipedia links for Ambient Techno and Hardstyle are retained
  only as lead-generating background and cannot be the sole support for a
  definition or candidate.
- **Contested:** `Minimal`, UK `Garage`, canonical `Electro`, cross-style
  `Acid`, `Tech House`, `Trance`, `Downtempo`, and `IDM` need bounded corpus
  definitions. Canonical `Dub` still needs a clear boundary from `Reggae`,
  `Dub Techno`, and generic dub mix/version naming. The reference dossier must preserve
  exclusions and boundary cases rather than silently choosing a retailer's
  current category meaning.
- **Unresolved:** the note's classifier-facing production hypotheses remain
  proposals unless the current-code statements explicitly say otherwise. No
  absence of correction should be read as fresh verification of every 2026-06
  sentence.

## Research Method

I treated sources differently depending on what kind of claim they support:

- **Strong source for tempo / broad traits:** Ableton Learning Music, Berklee,
  Beatport/Beatportal, and Roland Articles.
- **Strong source for production-feature hypotheses:** producer-oriented
  magazines with concrete pattern breakdowns. These are useful for DSP feature
  ideas, but not sufficient alone for genre-wide tempo ranges.
- **Useful but lower authority:** production tutorials and genre-guide sites when
  they describe practical signal-level details that match stronger historical
  sources.
- **Weak evidence:** Reddit, forums, private taxonomies, and unsourced genre
  pages. I avoided relying on these for classifier recommendations unless the
  same point is supported elsewhere.

Important meta-point: Beatport itself warns that current genre names reflect how
DJs and scenes categorize music now, and may not match historical definitions.
This makes Beatport useful as genre-taxonomy research, but not immutable
musicological truth or a required runtime evidence source. See
[Beatport's house history](https://www.beatportal.com/articles/29020-beatports-definitive-history-of-house-music)
and the current
[Beatport delivery genre list](https://greenroomsupport.beatport.com/hc/en-us/articles/41043520429076-Beatport-Genres-Including-NEW-Open-Format-Genres).

## High-Level Classifier Conclusions

1. **BPM is a useful prior, not a genre definition.** Ableton gives common tempo
   anchors: dub 60-90, hip-hop 60-100, house 115-130, techno/trance 120-140,
   dubstep 135-145, drum and bass 160-180 BPM. Berklee gives broader ranges for
   house and techno (120-150) and confirms D&B at 160-180 and dubstep around
   140. Use BPM to penalize impossible candidates and to split large families,
   but avoid making BPM the only positive signal.
   Sources: [Ableton tempo and genre](https://learningmusic.ableton.com/make-beats/tempo-and-genre.html),
   [Berklee electronic genre guide](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles).

2. **Kick pattern is the highest-leverage underused discriminator.** Four-on-floor
   versus broken, two-step, half-time, sparse, and irregular patterns strongly
   helps separate House/Techno/Trance/Hard Dance from Electro, Breakbeat,
   Dubstep, Jungle, D&B, Footwork, Trip-Hop, and much of UKG. It is not binary:
   Bassline/Niche, Speed Garage, Electro House, and some hard-dance forms are
   also 4/4. Stratum now supplies `kick_pattern` and
   `kick_pattern_confidence`, but the current classification engine does not
   convert them into genre votes. Any future rule should combine pattern with
   tempo, metadata, and timbre.

3. **Dub lineage is not enough by itself.** Dub, Dub Techno,
   Dubstep, Future Garage, Ambient Techno, and Trip-Hop all share echo, reverb,
   bass, space, or dub production lineage. Tempo and rhythm must split them:
   slow reggae/dub riddim, 4/4 techno pulse, 140 half-time dubstep, 2-step/UKG
   syncopation, or slow breakbeat.

4. **Atonal/dark/long-tail signals are defensible for Techno depth, but they are
   safest as conjunctions.** Techno is defined around repetition, rhythm,
   machines, and often non-lead-instrument texture. Dub Techno and Ambient
   Techno sources specifically support reverb, delay, pads, spatial depth, and
   subtle evolution. These justify `Atonal`, `LongTail`, low centroid, and
   dub-stab features. As classifier policy they should be conjunctive or
   explicitly accepted single-flag heuristics; current code still has some
   single-flag Techno-depth preferences that should be tightened or defended
   with calibration examples.

5. **Metadata remains essential for vocal/cultural styles.** Gospel House,
   Dancehall, Grime, Hip Hop, R&B, Highlife, Pop, Rock, Jazz, and Reggae contain
   traits that audio descriptors may not reliably infer without vocal/lyric or
   instrumentation models. Provider tags, label maps, artist context, and current
   tags should remain stronger than generic energy/timbre rules for these.

## Current Taxonomy Pressure Points

### BPM Ranges

The repo ranges mostly align with current guides once the classifier's
`BPM_TOLERANCE = 5` is considered.

Potential review items:

- **Deep House:** repo range is 118-126. LSA says 110-125 for deep house, while
  Ableton's broad house range is 115-130. With the current +/-5 BPM tolerance,
  113-117 BPM already remains plausible; the source-supported gap is mainly
  110-112 BPM. Lowering the stored minimum to 110 would also make 105-109 BPM
  plausible unless tolerance handling changes, so validate against the user's
  library before widening.
- **Tech House:** repo range is 124-132. LSA gives a narrow 122-125 but also
  says it can vary within house BPM. Given Beatport's current DJ-facing usage,
  the repo range is reasonable.
- **Techno:** repo range is 128-140. Sources give 120-150 broadly. Because the
  taxonomy has Minimal, Deep Techno, Dub Techno, Ambient Techno, Hard Techno, and
  Acid as separate entries, the narrower generic Techno range is defensible as a
  center-lane range. The +/-5 tolerance already admits 123-145.
- **Hardcore / Happy Hardcore:** LSA gives 160-200 for both. Repo maxes at 180.
  This avoids sucking uptempo/frenchcore into a taxonomy that does not name them,
  but it means very fast hardcore will be BPM-penalized.
- **Drum & Bass:** sources consistently give 160-180 BPM. The repo range is
  168-180, which with tolerance admits 163-185 but still penalizes 160-162 BPM.
  Review whether this is an intentional center-lane choice or should widen to
  source-consistent `160-180`.

### Family Boundaries

- **Electro in `GenreFamily::Techno` is risky without kick-pattern evidence.**
  Canonical Electro means 808/electro-funk/hip-hop/drum-machine lineage, not
  4/4 Electro House. Same-family resolution should not use atonality alone to
  drag Electro into Deep Techno. This is an existing classifier risk because
  Electro is currently in the Techno family; until kick-pattern validation is
  wired, consider excluding Electro from generic Techno-depth resolution or
  requiring explicit broken/syncopated Electro evidence before demoting it.

- **UK Garage styles straddle House and Bass.** `Garage`, `2-Step Garage`,
  `Speed Garage`, `Bassline`, `UK Funky`, and `Future Garage` are rhythmically
  closer to a UKG/Bass continuum than straight House. A future family split may
  be cleaner, but kick-pattern + BPM can handle most cases for now.

- **Hard Trance is trance-derived but currently code-classified with Hardcore.**
  Treat that as a pragmatic hard-dance placement, not a statement that Hard
  Trance should resolve like Hardcore without melodic/breakdown evidence.

- **Acid is source-specific, not a family by itself.** A TB-303 style signal can
  support Acid, Acid House, Acid Techno, Hard Trance, or Psytrance contexts. The
  current canonical `Acid` entry should require strong 303-like metadata or audio
  evidence. Current code maps the single token `acid` directly to `Acid`, so
  future hardening should either weaken that token or require corroboration from
  provider/label metadata or a future 303-like proxy.

## Classifier Feature Implications

### Candidate Rules Or DSP Features

| Feature | Genres helped | Source support | Implementation note |
|---|---|---|---|
| Kick pattern: four-on-floor vs broken vs half-time vs sparse | Electro, Breakbeat, 2-Step, Garage, Dubstep, D&B, Jungle, Footwork, House/Techno | Strong | Prioritize the existing kick-pattern plan. It encodes major rhythm distinctions, but 4/4 is not a clean family separator by itself. |
| Dub chord/stab detector | Dub Techno; Tech House false-positive analysis | Strong for Dub Techno; weak/contextual for Tech House | Supported by Ableton and Attack Magazine dub-techno material. Must remain conjunctive because some Tech House/House uses stabs too. |
| Long-tail reverb/delay decay | Dub Techno, Ambient Techno, Dub, Ambient | Strong | Useful positive signal when combined with tempo/rhythm/family evidence. |
| Band-split flux / sustained texture detection | Deep Techno vs Ambient | Moderate source support; deferred/speculative implementation | Drone sources support sustained tones and slow harmonic change; band-split flux is a practical DSP proxy, but the current roadmap defers it until nearer-term features are validated. |
| Sub-bass prominence | Dubstep, D&B, Jungle, Footwork, Dub | Strong | Use with rhythm pattern; bass alone collides across many genres. |
| Half-time or snare-on-3 feel | Dubstep; some Footwork and 140-adjacent bass cases | Strong for Dubstep; contextual elsewhere | Do not count half-time feel as positive D&B/Jungle/Dub evidence by itself. |
| Distorted/pitched kick profile | Hard Techno, Hardcore, Gabber, Hardstyle | Strong | Consider a hard-dance kick feature before adding more hard-dance rules. |
| TB-303 / acid pattern proxy | Acid | Moderate | Audio detection is difficult but metadata and label/source terms are highly diagnostic. Treat direct `acid` token matches as weak until corroborated. |

### Weak Or Dangerous As Rules

| Signal | Why dangerous |
|---|---|
| Generic "dark" timbre | Collides across Dub Techno, Deep Techno, Ambient, Drone, EBM, Dubstep, Trip-Hop. |
| BPM alone inside House/Techno | Sources give overlapping 120-140/150 ranges. |
| "Dub" text alone | Could mean canonical Dub, Dub Techno, Dubstep, Future Garage, or just dub mix/version. |
| Broad "Electronic/Electronica" provider tags | Beatport/Discogs often use these as umbrella/listening categories. The repo correctly omits vague tokens like "electronic". |
| Single vocal presence | Gospel House, House, Dancehall, Grime, Hip Hop, R&B, Pop, Trance, Happy Hardcore all may feature vocals. |

## House Family

### House

Supported traits:

- Four-on-floor / 4/4 dance rhythm, drum-machine basis, club culture lineage.
- Typical broad tempo: 115-130 BPM per Ableton; Berklee gives 120-150 for house
  as a very broad electronic overview.
- Soulful vocals are a common historical marker, but not required.

Classifier implications:

- `FourOnFloor + 115-130 + tonal/chordal + dancefloor` is a valid broad House
  support pattern.
- House should not win over Techno solely because BPM is 124-128; use timbre,
  harmonic clarity, groove swing, vocals, and provider tags.

Sources:
[Ableton](https://learningmusic.ableton.com/make-beats/tempo-and-genre.html),
[Berklee](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[Beatportal house history](https://www.beatportal.com/articles/29020-beatports-definitive-history-of-house-music).

### Deep House

Supported traits:

- Slower house variant with muted basslines, spacious percussion, pads, chords,
  ambient/soulful quality, and often vocals.
- LSA gives 110-125 BPM. Berklee summarizes deep house as smooth/jazzy chords
  with soulful melody emphasis.

Classifier implications:

- Existing `Atonal` negative for Deep House is well justified. Deep House is
  chordal and soulful; a no-tonal-center track should prefer Techno-family or
  Downtempo candidates unless strong metadata says Deep House.
- Positive support should combine house tempo, four-on-floor, lower energy than
  peak House/Techno, harmonic clarity/key confidence, and provider or label
  context.

Sources:
[LSA deep house](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Berklee house subgenres](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles).

### Tech House

Supported traits:

- Hybrid of House and Techno, with house tempo, rawer techno-styled kick/drums,
  stripped repetition/groove, minimal and dub elements, less musical richness
  than Deep House.
- Beatportal emphasizes that Tech House has shifted historically and now spans
  multiple DJ-scene meanings.

Classifier implications:

- Do not collapse Tech House into either House or Techno by family alone.
- Useful positive signals: house-range BPM, four-on-floor, raw/short drum sound,
  repetitive groove, limited harmonic density, and some house or label support.
- Sidechain/pumping may help but should be validated; it is less directly
  supported by sources than kick/drum/groove claims.

Sources:
[Beatportal tech house history](https://www.beatportal.com/articles/60692-beatports-definitive-history-of-tech-house),
[LSA tech house](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Progressive House

Supported traits:

- Long-form melodic house, gradual builds, evolving textures, tension/release,
  atmospheric pads, emotive chord progressions.

Classifier implications:

- Needs arrangement-level features more than scalar track-level features:
  long intros/builds, breakdowns, gradual energy ramp, melodic/chordal material.
- Current BPM range 122-132 is plausible, but BPM alone cannot separate it from
  Tech House, House, or Trance.

Sources:
[EDMProd progressive house](https://www.edmprod.com/what-is-progressive-house/),
[Melodigging progressive house](https://www.melodigging.com/genre/progressive-house).

### Garage, 2-Step Garage, Speed Garage, Bassline, UK Funky

Supported traits:

- 2-Step uses syncopated kick patterns that skip beats and shuffled/triplet
  percussion, commonly 130-135 BPM.
- UK Garage uses percussive shuffled breakbeat/4x4 rhythms, syncopated hats,
  irregular kick patterns, and chopped/time-stretched vocals; commonly around
  130 BPM.
- Speed Garage takes UK garage/house lineage toward sped-up 4/4 rhythm, rolling
  snares, breakbeat overlays, warped/reverse heavy basslines, time-stretched or
  chopped vocals, and jungle/dub sound effects.
- Bassline/Niche uses 4x4 rhythm, UKG/Grime vocal influence, prominent bassline,
  commonly 135-142 BPM.
- UK Funky blends house with Afro/Latin percussion and syncopation around 130.

Classifier implications:

- These genres should be driven by kick-pattern and swing/syncopation features,
  not by the House family depth model.
- `Garage` vs `2-Step Garage`: require a missing/skipped kick or syncopated
  two-step pattern for 2-Step; 4x4/shuffle with vocal chops can stay Garage.
- `Speed Garage`: distinguish from 2-Step by 4x4 kick, rolling snare/breakbeat
  overlay, warped/reverse bassline, and older UKG/NY-house lineage; distinguish
  from Bassline by era/context and less single-minded bassline aggression.
- `Bassline`: require prominent bassline + UKG/Grime context and 135-142-ish BPM.

Sources:
[LSA 2-Step/Bassline/Garage/UK Funky](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[MasterClass speed garage](https://www.masterclass.com/articles/speed-garage-music-guide).

### Afro House, Gospel House, Disco, Italo Disco, Italodance

Supported traits:

- Afro House fuses house with African percussion, vocals/chants, organic
  textures, deep bass, and polyrhythmic/indigenous instrumentation; tempo is
  usually in house territory but source ranges differ.
- Gospel House depends heavily on gospel vocal/choral/organ/piano signifiers
  inside a house groove.
- Disco: four-on-floor, syncopated basslines, strings/horns/electric piano/funk
  instrumentation, often 110-135 BPM. Modern disco is more DJ-grid friendly.
- Italo Disco: electronic/synth disco, vocoders, electronic keys/pianos, drum
  machine repetition, often English/European pop vocals.

Classifier implications:

- These are metadata/instrumentation/vocal-heavy. Do not overfit scalar DSP rules.
- For Afro House, use percussion density/polyrhythm and provider tags; avoid
  confusing with UK Funky or Tribal House-like tags if taxonomy remains flat.
- For Gospel House, label/artist/release/source tags are likely more reliable
  than audio features unless a vocal/organ classifier exists.

Sources:
[LSA Afro House/Disco/Italo](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Beatportal Afro House production](https://www.beatportal.com/articles/647491-step-by-step-guide-to-creating-an-afro-house-track-keinemusik-black-coffee-caiiro-alex-wann-style),
[Toolroom gospel house](https://toolroomrecords.com/news/the-gospel-of-house/),
[Pressure Radio gospel house](https://pressureradio.com/gospel-house/).

## Techno Family

This section follows the research grouping of techno/trance-adjacent dance
genres. Code-family exceptions, especially `Hard Trance`, are called out where
they matter for classifier behavior.

### Techno

Supported traits:

- Repetitive four-four beats for continuous layered DJ sets; rhythm/repetition
  emphasized over conventional lead instruments.
- Machine palette: drum machines, synthesizers, 808/909/303/SH-101/DX-era
  electronic instrumentation.
- Broad tempo: 120-150 BPM in LSA/Berklee; Ableton groups techno/trance at
  120-140.

Classifier implications:

- `FourOnFloor + regular rhythm + 123-145 effective BPM + machine timbre` is
  good broad support.
- Techno should win over Deep Techno when energy is high or the track lacks
  long-tail/dark/atonal/deep signals.

Sources:
[LSA techno](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Berklee techno](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles).

### Minimal

Supported traits:

- Stripped-down Techno/House lineage, subtle loop changes, hypnotic repetition,
  sparse arrangement. Berklee directly supports minimal techno as a techno
  subgenre; LSA's Minimal House/Micro House section is adjacent evidence only,
  emphasizing cushiony kick, hats, synthetic strings, dreamy keyboard tones, and
  115-130 BPM.

Classifier implications:

- Use low event density, high repetition, restrained dynamics, and provider/label
  evidence.
- Minimal can collide with Deep Techno and Dub Techno. Avoid using low complexity
  as a direct Minimal rule when low centroid or long-tail strongly supports Dub
  Techno/Deep Techno.

Sources:
[Berklee minimal techno](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[LSA minimal house/micro house](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Deep Techno

Supported traits:

- There is less direct high-authority genre-guide material for "Deep Techno" as
  a distinct taxonomy item. The best-supported inference combines Techno's 4/4
  repetition and machine rhythm with dark/deep/hypnotic/minimal traits, lower
  brightness, long-form evolution, and less overt melody.

Classifier implications:

- Deep Techno should be a conjunctive classification:
  `Techno-family evidence + 4/4 + 120-132-ish BPM + regular rhythm + dark/low
  centroid or long-tail + low melodic salience / modal-minor harmony / ambiguous
  key confidence + not high-energy`.
- Treat `Atonal` as weak by itself. It should only support Deep Techno when
  paired with techno pulse plus atmospheric, long-tail, noise, or metadata
  evidence. Do not let a single low centroid or atonal flag override strong Dub
  Techno, Electro, Ambient, or Downtempo evidence.

Sources:
The inference is grounded in [LSA techno](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Berklee techno/minimal](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
and the Dub/Ambient Techno sources below.

### Dub Techno

Supported traits:

- Strongest markers: dub chords/stabs, echo, delay, reverb, spatial depth,
  fuzzy/dark/melancholic pads, one repeated chord or minor-key material, 4/4
  kick and off-beat hat, and slow subtle evolution.
- Attack Magazine's Basic Channel-style breakdown explicitly starts from a
  four-to-the-floor kick and off-beat hat, then adds dub-delayed chords, filtered
  delay, reverb, and sidechain so the kick cuts through textures.
- Pitchfork's Basic Channel review identifies delay/reverb, murk, softened
  kicks, spatial dub techniques, and slowly revealing dense structures as core
  to the defining compilation.

Classifier implications:

- The existing dub-stab cache fields (`dub_stab_onset_rate`,
  `dub_stab_template`, `dub_stab_template_score`) are defensible ingredients for
  future Dub Techno templates, but are not currently threaded into
  `AudioFeatures` or wired as a direct classifier rule. Any derived dub-stab
  score should combine with 4/4 techno pulse, dark timbre/low centroid,
  long-tail decay, techno-compatible BPM, and Techno-family metadata.
- Use BPM as a soft prior for Dub Techno, not a narrow gate. The cited Attack
  production example uses 145 BPM, and broad Techno sources allow roughly
  120-150 BPM. A 120-145/150-ish window is more defensible than 118-132 as a
  genre-wide rule.
- Dub Techno should beat Deep Techno when chord/stab/delay evidence is present.
- Dub Techno should lose to Dubstep when half-time/sparse 140 rhythm and wobble
  sub-bass are present, and to Dub at 60-90 with reggae riddim.

Sources:
[Ableton dub chords](https://www.ableton.com/en/blog/designing-dub-chords-in-ableton-live-with-el-chooppizza-hotline/),
[Attack Magazine Basic Channel-style dub techno](https://www.attackmagazine.com/technique/beat-dissected/basic-channel-style-dub-techno/),
[Pitchfork Basic Channel BCD review](https://pitchfork.com/reviews/albums/basic-channel-bcd/),
[Pheek dub techno guide](https://audioservices.studio/production/pheeks-dub-techno-making-guide).

### Ambient Techno

Supported traits:

- Ambient textures plus techno rhythm/production. Sources emphasize layered
  atmospheres, reverbed pads, subtle drum programming, de-emphasis of sampling,
  and album/listening contexts.
- Ambient Techno also includes Gas-like combinations of lush atmospheres and
  4/4 minimal techno beats.

Classifier implications:

- Positive pattern: techno-compatible tempo, subtle/sparse drums, low to medium
  danceability, high atmospheric/long-tail texture, pads, and not enough
  dub-stab evidence for Dub Techno.
- If the beat is absent or too weak, prefer Ambient over Ambient Techno.

Sources:
[Ambient techno overview](https://en.wikipedia.org/wiki/Ambient_techno),
[Berklee ambient](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles).

### Acid

Supported traits:

- Acid house/techno is strongly associated with Roland TB-303 bassline
  synthesizer patterns, resonant squelch, filter movement, slides, accents, and
  4/4 club rhythm.

Classifier implications:

- A future 303 proxy would help, but metadata is probably more reliable in the
  near term.
- Treat `Acid` as a specific sound-source marker. Avoid classifying as Acid from
  tempo/energy alone, and treat a bare `acid` text token as weak unless
  corroborated by provider, label, release-title, or future 303-proxy evidence.

Sources:
[MasterClass acid house](https://www.masterclass.com/articles/acid-house-music-guide),
[Roland TB-303 article](https://articles.roland.com/beyond-acid-pushing-the-tb-303-into-new-sonic-territory/),
[Berklee house subgenres](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles).

### Electro

Supported traits:

- Canonical Electro/Electro Funk is funk and early hip-hop lineage, TR-808 drum
  machine/electronic sounds, synths, drum machines, rap/turntablism/early hip-hop
  context. LSA also has a separate Electro House definition; do not confuse it
  with canonical Electro.

Classifier implications:

- Kick-pattern detector should be the primary audio discriminator: Electro
  should be broken/syncopated/808-funk rather than four-on-floor Techno.
- If metadata says Electro but the kick pattern is four-on-floor and other
  evidence points to Electro House or Tech House, consider whether the taxonomy
  mapping is wrong.

Sources:
[LSA Electro Funk and Electro House distinction](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Trance, Psytrance, Hard Trance

Supported traits:

- Trance: repetitive/melodic uplifting energy, long builds, emotional breakdowns,
  euphoric drops, usually 125-150 or 135-150 depending source.
- Psytrance: faster trance variant with hypnotic beats, rolling basslines,
  psychedelic layers, cosmic effects, around 140-150.
- Hard Trance: hard kicks/resonant basses, reverberated beat, trance lineage,
  140+ range in guides; repo uses 138-150.

Classifier implications:

- Trance should require melodic/build/breakdown evidence or provider support;
  BPM and 4/4 alone collide with Techno.
- Psytrance can use high regularity, 138-148 BPM, rolling-bass proxies if/when
  available, and psychedelic/Goa/psy metadata. Rolling bass is plausible and
  production-supported, but source support is weaker than the tempo/trance
  lineage claim, so validate before making it a strong rule.
- Hard Trance sits closer to hard dance; distinguish from Hardstyle/Hard Techno
  by trance melodic/breakdown structure and less pitched/distorted hardstyle kick.
- Current code places `Hard Trance` in `GenreFamily::Hardcore`, even though it is
  musicologically trance-derived. Treat that as a pragmatic hard-dance placement:
  future rules should require trance melodic/breakdown evidence before preferring
  Hard Trance over Hardcore/Hardstyle, or consider a future `HardDance` family.

Sources:
[Berklee trance](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[LSA trance / hard trance / Goa](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[MasterClass trance subgenres](https://www.masterclass.com/articles/what-is-trance-music),
[PromoLink psytrance](https://promolink.app/blog/what-is-psytrance),
[UnderOwl psytrance](https://underowl.net/en/genre/psytrance),
[Daniel Sokolovskiy psytrance bassline synthesis](https://dsokolovskiy.com/blog/all/psytrance-bassline-synthesis/).

### EBM

Supported traits:

- Electronic Body Music draws on industrial and punk; repetitive industrial
  beats, disco/rock rhythms, gothic/punk/new wave overtones.

Classifier implications:

- Audio rules need caution: EBM can overlap Techno, Synth-pop, Industrial, and
  Electro. Use label/artist/provider evidence, dark industrial timbre, vocal
  style, and lower dance tempos.

Sources:
[LSA EBM](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Hard Techno

Supported traits:

- 140-160 BPM, heavy distorted kicks, minimal but forceful percussion, sharp
  offbeat hats, aggressive/industrial/dissonant synths, distortion/saturation.

Classifier implications:

- Current repo range 145-160 is reasonable for a center-lane classifier.
- Add a distorted-kick/aggressive transient feature before tuning Hard Techno
  rules much further.
- Distinguish from Hardcore/Gabber by slightly lower BPM and techno arrangement
  continuity; distinguish from Hardstyle by less pitched/euphoric kick-melody
  and fewer anthem/trance lead structures.

Sources:
[EDMProd hard techno](https://www.edmprod.com/what-is-hard-techno/),
[LSA techno/hard dance sections](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

## Bass Family

### Drum & Bass and Jungle

Supported traits:

- D&B: 160-180 BPM, syncopated breakbeats, intricate drum patterns, layered
  samples, deep sub-bass, sleeker/polished rave production.
- Jungle: similar tempo, rapid breakbeats, heavily syncopated loops, dub/reggae/
  dancehall/ragga vocal and bassline lineage, rawer feel, frequent Amen or funk
  break sampling.

Classifier implications:

- `Fast + BrokenBeat + sub-bass` is strong family support.
- D&B vs Jungle likely needs metadata and timbral/production differences:
  ragga/reggae/dub vocal samples and raw break texture support Jungle; polished
  high-energy production supports D&B.
- Repo BPM range review: `Drum & Bass` is currently 168-180, narrower than the
  160-180 range given by Ableton/Berklee/LSA. Decide whether the narrower range
  is intentional center-lane behavior or whether 160-162 BPM D&B should remain
  BPM-plausible.

Sources:
[Berklee D&B/Jungle](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[LSA D&B/Jungle](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Dubstep

Supported traits:

- Early London dubstep: around 140 BPM, sparse syncopation, dramatic drops,
  prominent modulated bass/sub-bass, half-time/double-time feel, snare backbone
  often on the third beat.

Classifier implications:

- Strong positive: 136-144 effective BPM, half-time feel, sparse syncopated
  drums, snare-on-3, sub-bass/wobble/modulation.
- Distinguish from Grime by less MC/vocal focus; from Dub Techno by half-time
  sparse rhythm rather than four-on-floor techno pulse.

Sources:
[Berklee dubstep](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[LSA dubstep](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Breakbeat and Broken Beat

Supported traits:

- Breakbeat: chopped/spliced/remixed sampled drum breaks from funk/post-disco/
  turntablism culture; broad 110-150 BPM.
- Broken Beat/Bruk: late-1990s West London scene; syncopated rhythms, heavy
  bass, shifting time signatures, jazz-funk/fusion/Afrobeat/dub/Latin influences,
  choppy grooves.

Classifier implications:

- `BrokenBeat` kick pattern is necessary but too broad. Breakbeat should be a
  generic fallback when tempo/rhythm are broken but D&B/Jungle/Dubstep/Garage/
  Footwork do not fit.
- Broken Beat needs metadata or jazz/funk instrumentation; otherwise audio may
  confuse it with Breakbeat, Future Garage, UK Funky, or IDM.

Sources:
[LSA breakbeat](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Roland early broken beat](https://articles.roland.com/listening-guide-early-broken-beat/),
[MusicRadar broken beat](https://www.musicradar.com/how-to/the-beginners-guide-to-broken-beat).

### Footwork

Supported traits:

- Chicago footwork/juke is a dance culture and electronic genre tied to ghetto
  house, usually around 150-165/160 BPM, with hyper-rhythmic offbeat/syncopated
  drum-machine patterns, cut-up samples/phrases, sub-bass, and frequent
  full-time/half-time abstraction. The Era source strongly supports culture and
  tempo; Planet Mu is stronger support than Wikipedia for the detailed audio
  traits.

Classifier implications:

- Strong positive pattern: 155-165 BPM, sparse/irregular beat-skipping kick
  pattern, vocal chops/sample fragments, sub-bass, high rhythmic abstraction.
- Avoid classifying all 160 BPM broken-beat music as Footwork: D&B/Jungle share
  the speed but use more rolling breakbeat structures.

Sources:
[The Era Footwork Collective](https://www.theerafootworkcollective.com/about-chicago-footwork),
[Planet Mu Bangs & Works Vol.1](https://planet.mu/releases/various-artists-bangs-works-vol-1/).

### Grime

Supported traits:

- Around 140 BPM, rapid syncopated 2-step and 4/4 breakbeats, aggressive/jagged
  electronic sound, strong MCing/inner-city lyric focus.

Classifier implications:

- Use MC/vocal/provider metadata heavily.
- Audio-only positive is weak unless future vocal/MC detection exists:
  140 BPM + broken/jagged drums + sparse synthetic bass can support Grime, but
  collisions with Dubstep, UK Garage, Bassline, and Breakbeat are high.

Sources:
[LSA grime](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Beatport genre list](https://greenroomsupport.beatport.com/hc/en-us/articles/41043520429076-Beatport-Genres-Including-NEW-Open-Format-Genres).

### Future Garage

Supported traits:

- UK Garage/2-Step beat plus atmospheric/futuristic soundscapes, pads,
  arpeggiated chords, vocoders, ambient sounds; commonly 130-140 BPM.

Classifier implications:

- Positive pattern: 2-step/UKG rhythm + lower/atmospheric texture + 125-138-ish
  BPM + low-to-medium dancefloor energy.
- Distinguish from Dubstep by less half-time snare/drop emphasis; from Ambient
  Techno by UKG rhythm rather than techno pulse.

Sources:
[LSA Future Garage](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

## Hardcore Family

### Hardcore and Gabber

Supported traits:

- Hardcore: fast tempo, intense kicks and synthesized bass, saturation,
  overdrive, industrial-like aggression; commonly 160-200 BPM.
- Gabber: very fast, distorted kick drums, shouted/roared vocals, punk/hardcore
  lineage; LSA says typically 180-190.

Classifier implications:

- Use high BPM and distorted-kick profile; Gabber should require stronger
  distortion/violence/rawness or provider metadata than generic Hardcore.
- Current taxonomy does not include Frenchcore/Uptempo; very fast tracks may
  need Manual or Hardcore with a low/medium confidence caveat.

Sources:
[LSA Hardcore/Gabber](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Hardstyle

Supported traits:

- Around 150 BPM modern center, distorted/overdriven hard kick, reverse bass,
  pitched kick/bass tone, euphoric saw leads, cinematic/emotional vocals.

Classifier implications:

- A hardstyle rule needs kick timbre/pitching, not just BPM. 150 BPM can also be
  Hard Techno, Psytrance edge, Drum & Bass halftime confusion, or fast Trance.
- Separate Hardstyle from Hardcore/Gabber by more melodic/euphoric lead
  structure and reverse-bass/pitched-kick behavior.

Sources:
[Hardstyle overview](https://en.wikipedia.org/wiki/Hardstyle),
[EDMProd hardstyle](https://www.edmprod.com/what-is-hardstyle/).

### Happy Hardcore

Supported traits:

- Fast hard beats, harsh basslines, powerful synthesizer breakdowns, high-pitched
  vocals, cartoonish/euphoric feel; commonly 160-200 BPM.

Classifier implications:

- Use high BPM + bright/euphoric melodic/vocal content + hard-dance kick; avoid
  identifying purely dark/aggressive 170 BPM music as Happy Hardcore without
  strong metadata.

Sources:
[LSA Happy Hardcore](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

## Downtempo / Ambient Family

### Ambient

Supported traits:

- Emphasizes soundscape, mood, texture, sustained tones, reverb, and often lacks
  clear tempo/key/beginning/end. Less attention to repetitive beats.

Classifier implications:

- `NonDancefloor + high atmospheric texture + no reliable beat` is well
  supported.
- Avoid using "low centroid" alone: Dub, Dub Techno, Dubstep, and Trip-Hop can
  also be dark/low.

Sources:
[Berklee ambient](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[MasterClass ambient guide](https://www.masterclass.com/articles/ambient-music-guide).

### Downtempo

Supported traits:

- Slow, hypnotic, ambient beats; chill-out room / lounge context; typically not
  more than 120 BPM.

Classifier implications:

- Existing non-dancefloor/slow vetoes are broadly justified.
- Distinguish from Trip-Hop by lack of slowed breakbeat/hip-hop influence; from
  Ambient by presence of a beat/groove.

Sources:
[LSA downtempo](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Trip-Hop

Supported traits:

- Fusion of hip-hop and electronica; slow bass-heavy breakbeat, ambient
  psychedelia, melancholic/reflective atmospheres, often vocals and jazz/soul
  instrumentation; typically not more than 120 BPM.

Classifier implications:

- `Slow + BrokenBeat + bass-heavy + atmospheric` supports Trip-Hop, but vocal
  and metadata evidence should matter because Downtempo and abstract Hip Hop
  overlap heavily.

Sources:
[LSA Trip Hop](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

### Dub

**Taxonomy decision (2026-07-18):** the reopened sources do not establish `Dub`
and `Dub Reggae` as two independently selectable genres. The
[London Museum](https://www.londonmuseum.org.uk/collections/london-stories/dub-london-shops-sound-systems-legends/)
uses “dub” and “dub reggae” as alternative names. The
[Jamaica Information Service](https://jis.gov.jm/jamaica/dub/) and
[Wesleyan University Press](https://www.weslpress.org/9780819565723/dub/)
define Dub as a style or subgenre that grew from Jamaican reggae. A
[peer-reviewed history indexed by Lancaster University](https://research.lancaster-university.uk/en/publications/king-tubby-meets-the-upsetter-at-the-grass-roots-of-dub-some-thou/)
uses “dub reggae” in its title while defining “Dub” as the same deconstructive
studio practice. These sources support a Reggae-versus-Dub boundary, but not a
Dub-versus-Dub-Reggae split. The operator retained `Dub` as canonical and made
`Dub Reggae` and `Reggae Dub` compatibility aliases. This keeps one coherent
candidate dossier without erasing existing input terminology.

Supported traits:

- Dub is stripped/reworked reggae/riddim, often partly instrumental, with vocals
  removed or fragmented; rhythm section emphasized; echo and reverb are central.
- Ableton tempo anchor for Dub is 60-90 BPM.

Classifier implications:

- `60-90 BPM + reggae/dub rhythm + echo/reverb + bass-heavy + vocal/instrumental
  fragments` supports Dub.
- Dub Techno and Dubstep should be excluded by rhythm/BPM first.

Sources:
[Ableton tempo and genre](https://learningmusic.ableton.com/make-beats/tempo-and-genre.html),
[LSA Dub](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Pitchfork Basic Channel review on dub techniques](https://pitchfork.com/reviews/albums/basic-channel-bcd/).

### IDM and Experimental

Supported traits:

- IDM is a cerebral/experimental counterpoint to club-focused electronic music,
  with intricate rhythms, offbeat melodies, odd meters, and less formulaic
  structures. LSA explicitly says no set rules and notes the term is disliked.
- Experimental is broader still.

Classifier implications:

- Use metadata/provider/current tag evidence. Audio-only IDM/Experimental
  classification should usually be Low/Insufficient because "complex" and
  "non-dancefloor" collide with Ambient, Downtempo, Breakbeat, Footwork,
  Industrial, and sound-art material.

Sources:
[Berklee IDM](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles),
[LSA IDM](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres).

## Other / Cross-Taxonomy Genres

### Dancehall

Supported traits:

- Faster reggae/dub fusion known as riddim; repetitive beats, electronic drum
  machines/synthesizers, Casio-style digital riddims, MC/toasting/Jamaican rap;
  commonly 90-108 BPM though faster forms exist.

Classifier implications:

- Use metadata/vocal cues strongly.
- BPM 90-108 and riddim rhythm can support Dancehall; avoid classifying slow
  Dub or Hip Hop as Dancehall without provider/current-tag support.

Sources:
[LSA Dancehall](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres),
[Electronic Beats dancehall riddims](https://www.electronicbeats.net/genre-guide-dancehall).

### Hip Hop, R&B, Pop, Rock, Jazz, Reggae, Highlife

These entries are useful for collection consistency but should be conservative
inside an electronic-genre classifier.

- Hip Hop: Ableton gives 60-100 BPM; rap vocals, 808s, sampled breaks, and
  vocal/lyric context matter more than generic tempo.
- R&B/Pop/Rock/Jazz are too broad for scalar electronic audio features.
- Reggae overlaps Dub/Dancehall but needs rhythm and vocal/instrumentation
  evidence.
- Highlife is West African popular music using horns, guitars, drums, and
  African rhythms; electronic variants exist but the taxonomy entry should be
  metadata-led.

Sources:
[Ableton hip-hop tempo](https://learningmusic.ableton.com/make-beats/tempo-and-genre.html),
[African Music Library highlife](https://www.africanmusiclibrary.org/genre/Highlife),
[MasterClass highlife](https://www.masterclass.com/articles/highlife-music-guide).

## Recommendations For `genre_classification`

1. **Keep the classifier evidence-first and auditable.** The sources confirm that
   scene terms are fluid. Evidence strings should explain whether a choice came
   from provider metadata, BPM plausibility, rhythm pattern, timbre, or a
   conjunctive template.

2. **Prioritize validated use of `kick_pattern`.** The descriptor is present in
   current evidence but is not a direct genre rule. It could unlock the cleanest
   decisions: Electro vs Deep Techno/Deep House, Dubstep vs Dub Techno,
   D&B/Jungle vs fast Techno/Hardcore, UKG/2-Step/Garage vs House, and Footwork
   vs Jungle/D&B. Treat any future use as high-leverage evidence, not a binary
   family separator, because some UKG, Bassline, Electro House, and hard-dance
   material is also 4/4.

3. **Treat dub-stab outputs as future conjunctive template ingredients.** The
   current cache exposes `dub_stab_onset_rate`, `dub_stab_template`, and
   `dub_stab_template_score`; these are not currently threaded into
   `AudioFeatures` as a classifier rule. The web research strongly supports dub
   stabs/delay/reverb for Dub Techno, but false positives in House/Tech
   House/Techno are expected. Any derived score should require Techno-family
   support, 4/4 pulse, long-tail/darkness, and plausible techno-compatible BPM.

4. **Add a hard-dance kick/timbre feature before major Hardstyle/Hardcore tuning.**
   Sources repeatedly identify distorted/pitched kicks, reverse bass, and
   aggressive saturation as more diagnostic than BPM alone.

5. **Consider a future UKG/Bass rhythmic family.** The current House/Bass split
   works for many genres, but UKG, 2-Step, Speed Garage, Bassline, UK Funky, and
   Future Garage form a rhythm continuum that the current families do not model
   cleanly.

6. **Do not expand vague tokens.** The repo's current omission of "electronic",
   "electronica", "dance", and similar umbrella terms is correct. Beatport and
   Discogs often use these as broad containers.

7. **Use `manual` / `insufficient` for broad listening genres where the audio
   surface is too generic.** Current Ambient/Downtempo audio vetoes are
   intentional conservative behavior and should be reviewed separately. The
   metadata-led/manual warning applies more strongly to IDM, Experimental, Pop,
   R&B, Jazz, Rock, and Highlife; these should not be forced from weak scalar
   audio evidence.

## Source Index

- [Ableton - Tempo and genre](https://learningmusic.ableton.com/make-beats/tempo-and-genre.html)
- [Berklee - Electronic Music Genres: A Guide to the Most Influential Styles](https://www.berklee.edu/berklee-now/news/electronic-music-genres-a-guide-to-the-most-influential-styles)
- [Beatport Greenroom - Beatport Genres](https://greenroomsupport.beatport.com/hc/en-us/articles/41043520429076-Beatport-Genres-Including-NEW-Open-Format-Genres)
- [Beatportal - Definitive History of House Music](https://www.beatportal.com/articles/29020-beatports-definitive-history-of-house-music)
- [Beatportal - Definitive History of Tech House](https://www.beatportal.com/articles/60692-beatports-definitive-history-of-tech-house)
- [Beatportal - Afro House production guide](https://www.beatportal.com/articles/647491-step-by-step-guide-to-creating-an-afro-house-track-keinemusik-black-coffee-caiiro-alex-wann-style)
- [London Sound Academy - List of Electronic Dance Music Genres](https://www.londonsoundacademy.com/blog/list-of-electronic-dance-music-genres)
- [MasterClass - Speed Garage Music Guide](https://www.masterclass.com/articles/speed-garage-music-guide)
- [Ableton - Designing Dub Chords](https://www.ableton.com/en/blog/designing-dub-chords-in-ableton-live-with-el-chooppizza-hotline/)
- [Attack Magazine - Basic Channel-Style Dub Techno](https://www.attackmagazine.com/technique/beat-dissected/basic-channel-style-dub-techno/)
- [Pitchfork - Basic Channel: BCD](https://pitchfork.com/reviews/albums/basic-channel-bcd/)
- [Pheek - Guide to Making Dub Techno](https://audioservices.studio/production/pheeks-dub-techno-making-guide)
- [Wikipedia - Ambient techno overview](https://en.wikipedia.org/wiki/Ambient_techno)
- [Thom Holmes - Electronic Drone Music](https://www.thomholmes.com/post/electronic-drone-music)
- [MasterClass - Ambient Music Guide](https://www.masterclass.com/articles/ambient-music-guide)
- [MasterClass - Acid House Music Guide](https://www.masterclass.com/articles/acid-house-music-guide)
- [MasterClass - Highlife Music Guide](https://www.masterclass.com/articles/highlife-music-guide)
- [MasterClass - Trance Music Guide](https://www.masterclass.com/articles/what-is-trance-music)
- [Roland - Beyond Acid: Pushing the TB-303](https://articles.roland.com/beyond-acid-pushing-the-tb-303-into-new-sonic-territory/)
- [Roland - Listening Guide: Early Broken Beat](https://articles.roland.com/listening-guide-early-broken-beat/)
- [MusicRadar - Beginner's guide to broken beat](https://www.musicradar.com/how-to/the-beginners-guide-to-broken-beat)
- [The Era Footwork Collective - About Chicago Footwork](https://www.theerafootworkcollective.com/about-chicago-footwork)
- [Planet Mu - Bangs & Works Vol.1](https://planet.mu/releases/various-artists-bangs-works-vol-1/)
- [EDMProd - What is Hard Techno?](https://www.edmprod.com/what-is-hard-techno/)
- [EDMProd - What is Hardstyle?](https://www.edmprod.com/what-is-hardstyle/)
- [EDMProd - What is Progressive House?](https://www.edmprod.com/what-is-progressive-house/)
- [Melodigging - Progressive House](https://www.melodigging.com/genre/progressive-house)
- [PromoLink - What Is Psytrance?](https://promolink.app/blog/what-is-psytrance)
- [UnderOwl - Psytrance genre guide](https://underowl.net/en/genre/psytrance)
- [Daniel Sokolovskiy - Psytrance bassline synthesis](https://dsokolovskiy.com/blog/all/psytrance-bassline-synthesis/)
- [Wikipedia - Hardstyle overview](https://en.wikipedia.org/wiki/Hardstyle)
- [Toolroom - The Gospel of House](https://toolroomrecords.com/news/the-gospel-of-house/)
- [Pressure Radio - Gospel House](https://pressureradio.com/gospel-house/)
- [Electronic Beats - Dancehall riddims](https://www.electronicbeats.net/genre-guide-dancehall)
- [African Music Library - Highlife](https://www.africanmusiclibrary.org/genre/Highlife)
