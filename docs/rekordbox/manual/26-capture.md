---
id: capture
title: "Extracting a section from a track (CAPTURE)"
type: manual
source:
  file: "rekordbox7.2.8_manual_EN.pdf"
  pages: "200-202, 206"
  version: "7.2.8"
topics: [pads, sampler, slicer]
modes: [performance]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

# Extracting a section from a track (CAPTURE)

You can extract a section from the loop play and slicer, and use it as a sampler.
For the sampler, see "Using the sampler deck" (page 188).

## To extract a section from loop playback (LOOP CAPTURE)

For the loop play, see "Starting loop play" (page 157).

1. Start the loop play from which you wish to extract a section.

   [Screenshot: Enlarged waveform showing a loop range highlighted in the upper and lower waveform display]

2. Click [<] on the right side of the enlarged waveform, and then click [capture icon].
   The color of the loop range changes.

   [Screenshot: Enlarged waveform with the loop range color changed, indicating capture mode is active]

3. Drag and drop the loop range to the slot of the sampler deck.

   [Screenshot: The loop range being dragged from the waveform down to a sampler slot on the sampler deck, with a dotted arrow showing the drag path]

   The range is ready to be used as a sampler.
   The sampler is stored at [Capture] in [Sampler] of [Media Browser].

**Note**

- When the [Preferences] window > [Controller] category > [Sampler] tab > [SLOT] > [Do not overwrite the loaded Slot] is selected, you cannot drag and drop to a slot already loaded.

## To extract a section from slicer (SLICER CAPTURE)

You can extract the whole range of the slicer or one of the 8 sections.
For the slicer, see "Using a slicer" (page 197).

### Extracting the whole slicing range

1. Select [SLICER] on the performance pad.

   [Screenshot: Enlarged waveform with slicer sections displayed as colored segments across the waveform]

2. Click [<] on the right side of the enlarged waveform, and then click [capture icon].
   The color of the slicer changes.

   [Screenshot: Enlarged waveform with the slicer color changed, indicating capture mode is active]

3. Drag and drop the waveform part of the slicing range to the 8 slots in either the right or left section of the sampler.

   [Screenshot: The slicing range being dragged from the waveform down to sampler slots, with a dotted arrow showing the drag path]

   The audio divided into 8 is loaded to eight sampler slots respectively, and ready to be used as a sampler.
   The sampler is stored at [Capture] of [Sampler] in [Media Browser].

**Note**

- When the [Preferences] window > [Controller] category > [Sampler] tab > [SLOT] > [Do not overwrite the loaded Slot] is selected, you cannot drag and drop to a slot already loaded.

### Extracting one of the 8 divided slicer sections

1. Select [SLICER] on the performance pad.

   [Screenshot: Enlarged waveform with slicer sections displayed as colored segments]

2. Click [<] on the right side of the enlarged waveform, and then click [capture icon].
   The color of the slicer changes.

   [Screenshot: Enlarged waveform with the slicer color changed, indicating capture mode is active]

3. Drag and drop the number part of the slicer section below the waveform to the slot of the sampler.

   [Screenshot: A single slicer section number being dragged from below the waveform to a sampler slot, with a dotted arrow showing the drag path]

   Ready to be used as a sampler.
   The sampler is stored at [Capture] of [Sampler] in [Media Browser].

**Note**

- When the [Preferences] window > [Controller] category > [Sampler] tab > [SLOT] > [Do not overwrite the loaded Slot] is selected, you cannot drag and drop to a slot already loaded.

# Using SAMPLE SCRATCH

Load the track in the sampler slot to the deck.

**Hint**

- To use SAMPLE SCRATCH, assign to the hardware on MIDI Learn or use keyboard shortcut.
  Set the followings from the [MIDI settings] window > [PAD] tab > [SampleScratch].
  - [SampleScratchMode]
  - [SampleScratch Pad1-8]
    Set the followings from the [Preferences] window > [Keyboard] category > [Deck 1] through [Deck 4].
  - [Pad mode - Sample Scratch]
  - [Pad A] through [Pad H]
    For details on how to operate MIDI Learn, refer to "MIDI LEARN Operation Guide" on the rekordbox website.
    For details on how to operate keyboard shortcuts, refer to "Default Keyboard shortcut references" on the rekordbox website.

## Using SAMPLE SCRATCH on DJ controller

1. Select [SAMPLE SCRATCH] from the Pad mode.

2. Press a performance pad.
   The sound in the sampler slot assigned to the pad is loaded to the deck, and then DJ performance such as scratching is available.
   - If [Play mode (Oneshot)] is set on the sampler slot, playback starts when the sound is loaded to the deck.
   - If [Play mode (Loop)] is set on the sampler slot, manual loop is set on the deck and playback starts when the sound is loaded to the deck.
   - If [Gate mode] is set on the sampler slot, the sound plays as Cue Point Sampler while holding the pad when the sound is loaded to the deck.

## Related Documents

- [manual/18-performance-screen.md](18-performance-screen.md) (pads, sampler, slicer)
- [manual/25-slicer.md](25-slicer.md) (pads, sampler, slicer)
- [faq/stems-and-effects.md](../faq/stems-and-effects.md) (sampler)
- [guides/pad-editor.md](../guides/pad-editor.md) (pads)
- [manual/23-sampler-deck.md](23-sampler-deck.md) (sampler)
- [manual/24-sequencer.md](24-sequencer.md) (sampler)
- [manual/27-active-censor.md](27-active-censor.md) (pads)
