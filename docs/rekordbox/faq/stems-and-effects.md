---
id: faq-stems-and-effects
title: "FAQ: STEMS & Effects"
type: faq
source:
  file: "rekordbox7-faq.md"
  url: "https://rekordbox.com/en/support/faq/rekordbox7/"
  version: "7.x"
topics: [effects, recording, sampler, sequencer, stems]
modes: [common]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

### Can I match the start position of DRUM CAPTURE to the BeatGrid?

You can set the start position of DRUM CAPTURE as the BeatGrid through the following method.

If the Deck is paused:

1. Set the Deck [Q] button (Quantize button) to ON.
2. Move the range you want to capture to the current position of the waveform.
3. Click the Deck [CUE] button.
   The closest BeatGrid is set to the current position.
4. Set the DRUM CAPTURE button to ON.
   The DRUM CAPTURE range is displayed from the current position.

If the Deck is playing:

1. Set the Deck [Q] button (Quantize button) to ON.
2. Set Auto Beat Loop (32 beats, etc.) to ON at the position you want to capture.
   A Loop from the closest BeatGrid (32 beats, etc.) is created.
3. The DRUM CAPTURE range is displayed from the Loop IN point.

---

### Please explain the condition DRUM SWAP stops automatically.

DRUM SWAP automatically stops when the deck playback stops or when BEAT SYNC to the deck cannot be maintained.

Examples when the deck playback stops:

- Click the [PLAY/PAUSE] button on the playing deck to pause playback.
- Click the [CUE] button on the playing deck to perform a Back Cue.
- The end of the track is reached on the playing deck.

Examples when BEAT SYNC to the deck cannot be maintained:

- Perform a MANUAL LOOP with quantize turned off on the deck.
- Perform reverse playback on the deck.

---

### Audio files captured on DRUM CAPTURE cannot be played.

Files with the extension ".rbsc" extracted from the DRUMS Stem can be played in the PERFORMANCE mode or LIGHTING mode of rekordbox for Mac/Windows under the account from which the DRUMS Stem was extracted.

---

### Audio files captured on DRUM CAPTURE cannot be exported.

Files with the extension ".rbsc" extracted from the DRUMS Stem can be exported in the PERFORMANCE mode or LIGHTING mode of rekordbox for Mac/Windows under the account from which the DRUMS Stem was extracted.

Extracted files cannot be used in the following:

- EXPORT mode and EDIT mode
- USB Export function
- LINK Export function
- CloudDirectPlay function
- rekordbox for iOS/Android

---

### I've accidentally deleted the Factory Preset for DRUM SWAP from the collection (or deleted the audio file). What can I do to restore it?

Preinstalled or additional download "Factory Presets" can be acquired through the following method.

- In the case of the preinstalled [Factory Presets] > [4-Floor / Breaks Kit]:

Reboot rekordbox.

- In the case of the additional download "Factory Presets":

Click [File]menu > [Additional Contents] > [GROOVE CIRCUIT FACTORY SAMPLES PACK] > [Download], download the GROOVE_CIRCUIT_FACTORY_SAMPLES_PACK.spp file,
click [File]menu > [Additional Contents] > [GROOVE CIRCUIT FACTORY SAMPLES PACK] > [Import], and select the downloaded GROOVE_CIRCUIT_FACTORY_SAMPLES_PACK.spp file.

---

### Where can I perform switching settings for Enlarged Waveform displays synchronized with the Stem selected in ACTIVE STEM?

ver. 7.0.4 or later:

The display switches through the [Preferences] > [Erweitert] category > [STEMS] tab > [STEM Waveform Display] ON/OFF setting.

Up to ver. 7.0.3:

The display switches each time the ![](https://cdn.rekordbox.com/files/20241003221616/enlargedwaveform.png) icon at the top of the Enlarged Waveform is clicked.

---

### How can I get better STEMS sound quality?

In Preferences below, select [Prioritize sound quality].

[Preferences] > [Extensions] category > [STEMS] tab > [STEMS Analysis Process]

*See the recommended CPUs listed [here](https://rekordbox.com/en/support/faq/stems-7/#faq-q700018) when using [Prioritize sound quality].

---

### Can GROOVE CIRCUIT be controlled through DJ equipment?

GROOVE CIRCUIT can be controlled using the following DJ equipment.

- DDJ-GRV6

(As of October 2024)

Each function can be used through mapping with the MIDI LEARN function on other DJ equipment. (The MIDI LEARN function requires a [Core plan subscription](https://rekordbox.com/en/support/faq/stems-7/#faq-q700019) or higher, or connection of [Hardware Unlock devices](https://rekordbox.com/en/support/faq/v7/#faq-q700001).)
See ["MIDI LEARN Operation Guide"](https://rekordbox.com/en/download/#manual) for mapping methods using the MIDI LEARN function.

---

### Using audio with no sound at the beginning on DRUM SWAP SLOT, the silent part is played as well.

DRUM SWAP SLOT plays audio from the beginning so the silent part is played as well.

If there is no sound at the beginning of the audio, you can remove the silent part using DAW software or EDIT mode (*). In the Free or Core plan, you can use only the track edit function. Rendering edited tracks as audio files requires the Creative or Professional plan. (Silent parts will be added at the beginning of audio for certain file formats such as MP3. Use the audio by removing the silent part and saving them in formats such as WAV, AIFF, etc.)

*See "[EDIT mode operation guide](https://rekordbox.com/en/download/#manual)" for how to use the EDIT function.

See [here](https://rekordbox.com/en/plan/) for details on plan.

---

### Quantize playing decks when playing DRUM SWAP SLOT?

Yes. Playing DRUM SWAP SLOT quantizes the beginning of audio within DRUM SWAP SLOT to match the deck beat position.

---

### When MIDI devices such as DJ controllers are connected, a key icon was applied, and GROOVE CIRCUIT became unavailable.

If MIDI devices other than supported controllers are connected, mouse controls and keyboard shortcuts become disabled on the GROOVE CIRCUIT function.

If subscribed to the plan, MIDI LEARN, mouse control, and keyboard shortcut controls become available.

Refer to [here](https://rekordbox.com/en/support/faq/stems-7/#faq-q700019) for GROOVE CIRCUIT supported DJ controllers.

---

### What is the GROOVE CIRCUIT function?

A remix function that uses DRUMS Stem of a track. You can replace the DRUMS Stem of a track with the drum loop audio source, extract the DRUMS Stem, or apply FX only to the DRUMS Stem.

- To enable the GROOVE CIRCUIT function, open the [Preferences] > [Extensions] category > [STEMS] tab, and check both the [Enable the STEMS Function] and [Enable the GROOVE CIRCUIT Function] checkboxes.
- To display the GROOVE CIRCUIT panel, click ![](https://cdn.rekordbox.com/files/20240912113247/GROOVE-CIRCUIT-icon.png) icon in the global section.

---

### rekordbox is slow when I'm using [Prioritize sound quality].

See the rekordbox [System requirements](https://rekordbox.com/en/download/#system) in addition to the following recommended CPUs before using [Prioritize sound quality].

- When using STEMS with 2 decks

Windows:

Intel® processor Core i7 (13th generation or later) / i9 (9th generation or later)
AMD Ryzen™7 5000 series or later

Mac:

Intel® processor Core i5 / i7 / i9 (with 2019 Mac or later)
Apple M1 series or later

- When using STEMS with 4 decks

Windows:

Intel® processor Core i9 (9th generation or later)
AMD Ryzen™7 7000 series or later

Mac:

Apple M1 series or later

*Operation is not guaranteed for all computers that meet the above system requirements.

---

### Can STEMS function be controlled by DJ equipment?

Yes, it can be controlled by DJ equipment on which STEMS function is mounted.

In addition, you can use each function by assigning it to the hardware on MIDI LEARN.

However, it is necessary to subscribe to a supported Plan or connect a HardwareUnlock device.

---

### If you use STEMS function, noise occurs.

Check the following.

1. Does your computer fulfill the environmental requirements of rekordbox?
   Check the [System Requirements](https://rekordbox.com/en/download/#system).
2. Is the buffer size properly adjusted?
   The latency (time that elapses before outputting sound after operating this equipment) can be shortened by setting the buffer size small.
   If audio dropout occurs, increase the buffer size. However, the latency becomes longer.
   Set the minimum buffer size that does not cause audio dropout.
   The buffer size can be adjusted by clicking [Preferences] > [Audio] category > [Configuration] tab > [Buffer size].

- If DJ equipment is connected, refer to the content shown in the operation manual.

3. Isn't other application operating?
   Close applications that are not in use including Web browser, Explorer (Windows) or Finder (Mac), screen saver, and resident software once.
4. You may be able to decrease the symptom by changing the setting of multithread.
   Click [Preferences] > [Extensions] category > [STEMS] tab > [Multi-thread] and deselect [Apply multi-thread to the analysis process].
5. You may be able to decrease the symptom by turning OFF the following functions with high processing load.

-
  - Video function
- Lyric function
- MERGE FX

---

### When loading tracks, a caution "The STEMS function is unavailable since there is not enough free space in the memory." is displayed.

Check the following.

1. Does your computer fulfill the environmental requirements of rekordbox?
   Check the System Requirements.
2. Is the buffer size properly adjusted?
   The latency (time that elapses before outputting sound after operating this equipment) can be shortened by setting the buffer size small.
   If audio dropout occurs, increase the buffer size. However, the latency becomes longer.
   Set the minimum buffer size that does not cause audio dropout.
   The buffer size can be adjusted by clicking [Preferences] > [Audio] category > [Configuration] tab > [Buffer size].

- If DJ equipment is connected, refer to the content shown in the operation manual.

3. Isn't an option for increasing the memory usage for analytical processing activated?
   Click [Preferences] > [Extensions] category > [STEMS] tab > [Memory] and deselect [Increase the memory size of the analysis process].

In addition, carrying out the following may resolve the insufficient memory space, however, change the setting at your own risk.

- Uninstall of unused application
- Deletion of unnecessary files

---

### What is STEMS function?

To activate STEMS function, check-mark [Enable the STEMS function] (Click [Preferences] > [Extensions] category > [STEMS] tab).

This is a function for outputting the sound of a track separately in VOCAL Stem, DRUMS Stem, BASS Stem, and INST Stem.

You can select either [3 Stems (VOCAL, INST, DRUMS)] or [4 Stems (VOCAL, INST, BASS, DRUMS)] in the [Preferences] > [Extensions] category > [STEMS] tab > [STEMS Mode].

ACTIVE STEM, STEM ISO, and STEM FX functions are usable.

- ACTIVE STEM
  Each Stem is output by setting each STEM MUTE button displayed at the deck Stem to ON. If it is set to OFF, each Stem is muted.
  Click [Preferences] > [Extensions] category > [STEMS] tab > [ACTIVE STEM Setting]. You can select [MUTE] or [SOLO].

With [MUTE] setting, output / mute of each Stem can be controlled.
With [SOLO] setting, output of all Stems or output of one Stem can be controlled.

- STEM ISO
  If you set STEM ISO mode button that is displayed at mixer Stem to ON, the mode shifts to STEM ISO mode in which you can adjust the sound volume of each Stem.
  When STEM ISO mode is ON, adjust the sound volume of each Stem with each knob.
- STEM FX
  If you set each STEM FX button that is displayed at effect Stem to ON, it is possible to apply an effect to each Stem.

---

### Where can I change between 3 Stems and 4 Stems?

You can change it in [Preferences] > [Extensions] category > [STEMS] tab, [STEMS mode].

---

### The sound of the recorded file is distorted.

The volume level of recording may be too high.

Turn the recording level knob. Adjust the recording level maximum to the level in which not all the meters hitting red.

Record again.

![](https://cdn.rekordbox.com/files/20200217192207/master_volumepng2-300x30.png)

---

## Related Documents

- [manual/18-performance-screen.md](../manual/18-performance-screen.md) (effects, sampler, sequencer, stems)
- [features/overview.md](../features/overview.md) (effects, stems)
- [manual/19-performance-preparing.md](../manual/19-performance-preparing.md) (recording, sequencer)
- [manual/24-sequencer.md](../manual/24-sequencer.md) (sampler, sequencer)
- [manual/28-stems.md](../manual/28-stems.md) (effects, stems)
- [manual/32-menu-list.md](../manual/32-menu-list.md) (effects, stems)
- [features/whats-new-v7.md](../features/whats-new-v7.md) (stems)
- [guides/lighting-mode.md](../guides/lighting-mode.md) (effects)
