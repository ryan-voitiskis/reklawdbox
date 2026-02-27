---
id: faq-streaming
title: "FAQ: Streaming"
type: faq
source:
  file: "rekordbox7-faq.md"
  url: "https://rekordbox.com/en/support/faq/rekordbox7/"
  version: "7.x"
topics: [streaming]
modes: [common]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

### What is TIDAL?

TIDAL is a streaming service that DJs can use to access the TIDAL music catalog.

A separate subscription is needed to use this service.

For details, check out the [TIDAL website](https://tidal.com/).

Click [here](https://support.tidal.com/hc/en-us) to access the TIDAL FAQ page.

---

### Which countries can TIDAL be used in?

For the latest information on countries and regions where TIDAL is available, check out the [TIDAL website](https://support.tidal.com/hc/en-us/articles/202453191-TIDAL-Where-We-re-Available).

---

### When I log into TIDAL, the login window doesn't display properly in my web browser. Why not?

You may be using a browser that's not supported by TIDAL.

Check the list of browsers that [TIDAL supports](https://support.tidal.com/hc/en-us/articles/115005872445-System-Requirements) and set one of those as your default browser.

---

### Why doesn't rekordbox show some of the tracks displayed on a streaming service's website or app?

The use of certain tracks on streaming services by third parties is restricted due to copyright holders' intentions or other reasons.

Tracks like these don't show up in rekordbox.

---

### Can I play Beatsource Streaming tracks on rekordbox even in an offline environment?

Yes. Select your desired tracks from the track list in the Beatsource category of the tree view and store them to rekordbox by choosing [Store Offline] to play them in an offline environment.

---

### What is SoundCloud Go+/DJ?

SoundCloud Go+/DJ is a subscription streaming service that DJs can use to access the SoundCloud music catalog.

A separate signing up is required to use this service.

For details, refer to the [SoundCloud](https://checkout.soundcloud.com/dj) website.

Click [here](https://help.soundcloud.com/hc/en-us/categories/115000706387-SoundCloud-Go) for the SoundCloud FAQ page.

---

### Do you support SoundCloud Go+/DJ High Quality Streaming?

High Quality Streaming can be used by accessing [Preferences] > [Advanced] > [Others] > [SoundCloud] and selecting [High Quality 256kbps (AAC)] for [Streaming Audio Quality].

However, depending on the tracks, the selected sound quality may not be available.

If Standard Quality is unavailable, the track will be analyzed and played back in High Quality; if High Quality is unavailable, it will be analyzed and played back in Standard Quality.

The BeatGrid and CUE points may also need to be reset due to the possibility of the BeatGrid and CUE points shifting when [Streaming Audio Quality] is changed.

---

### What types of file formats are used by streaming services?

The file formats used by each streaming service are as follows:

- Apple Music: AAC 256kbps
- Spotify: Ogg 96kbps, Ogg 320kbps
- Beatport Streaming: AAC 128 kbps, AAC 256 kbps, FLAC 44.1 kHz 16 bit (*1)
- Beatsource Streaming: AAC 128 kbps, AAC 256 kbps, FLAC 44.1 kHz 16 bit (*1)
- SoundCloud Go+: AAC 160kbps/96kbps, AAC 256kbps
- SoundCloud DJ: AAC 160kbps/96kbps, AAC 256kbps
- TIDAL (plan including DJ Extension): AAC 320 kbps, FLAC 44.1 kHz 16 bit

(*1) The file formats varies depending on your subscription plan.

For more details, check the website of each streaming service.

-
  - [Apple Music](https://www.apple.com/apple-music/)
- [Spotify](https://open.spotify.com/)
- [Beatport Streaming](https://stream.beatport.com/)
- [Beatsource Streaming](https://www.beatsource.com/)
- [SoundCloud](https://checkout.soundcloud.com/dj)
- [TIDAL](https://tidal.com/)

---

### What is the audio quality of the streaming services that rekordbox supports?

The following audio quality is supported by each streaming service.

- Apple Music

AAC 256kbps

- Spotify

Default (Ogg 320kbps)
Low (Ogg 96kbps)

- SoundCloud

High Quality 256kbps(AAC)
Standard Quality 160kbps or 96kbps (AAC)

- Beatport Streaming

Lossless(FLAC)
High(256 AAC)
Standard(128 AAC)

- Beatsource streaming

Lossless(FLAC)
High(256 AAC)
Standard(128 AAC)

- TIDAL

Lossless(FLAC) 44.1kHz 16bit
High(AAC) 320kbps

---

### The BeatGrids and Cue points on the streaming service are misaligned. How can I fix the misalignment?

BeatGrids and CUE points may need to be reset due to the possibility of the BeatGrids and CUE points shifting when [Streaming Audio Quality]* is changed.
*Access [Preferences] > [Advanced] categories > [Others] tab to set the [Streaming Audio Quality] for [Spotify], [SoundCloud], [Beatport], [Beatsource], or [TIDAL].
The [Streaming Audio Quality] settings are shared and applied on the devices that are logged in with the same Beatport account or Beatsource account.
Additionally, after updating, BeatGrids and Cue points may be misaligned for Apple Music tracks (analyzed with ver. 7.2.7 or earlier) and SoundCloud tracks (analyzed with ver. 7.2.9 or earlier).

If the BeatGrids and Cue points have shifted, reanalyze the tracks and reset the Cue points.

- For Spotify tracks, you cannot select the [Analyze Track] option in the context menu. Select [Remove from rekordbox] from the context menu first, then reload the track into the deck to have it reanalyzed. After that, set the CUE points again.

* BeatGrids and phrases
  Analyse the tracks.
  The BeatGrid and phrase positions can also be adjusted and changed manually.
  (For details, see "[GRID/PHRASE EDIT] panel" in the Instruction Manual.)
* For CUE points such as Hot Cues and Memory Cues
  Delete the CUE points before resetting them.

Hint: Method used to reanalyze multiple tracks at once

1. Select [Collection], click the icon on the [Attribute] column header, add a check to the checkbox for the streaming service with the tracks you wish to reanalyze, and click OK.
2. Select the tracks you wish to reanalyze (multiple can be selected at a time) and select [Analyze Track] from the right-click menu to reanalyze the tracks.

---

### BeatGrids and CUE points misaligned for SoundCloud tracks.

The audio quality and format of SoundCloud tracks have been changed.
Therefore, BeatGrids and CUE points analyzed prior to ver. 7.2.9 may be misaligned.

If you notice any misalignment, please reanalyze the tracks and reset the CUE points.
Also, changing the [Streaming Audio Quality] setting may cause BeatGrids and CUE points to misalign.

For details, please see [here](https://rekordbox.com/en/support/faq/streaming-7/#faq-q700146).

---

### I can't use tracks with the audio quality set in [Streaming Audio Quality] on SoundCloud.

Depending on the tracks, the selected sound quality may not be available.

If Standard Quality is unavailable, the track will be analyzed and played back in High Quality;
if High Quality is unavailable, it will be analyzed and played back in Standard Quality.

---

### What is Apple Music?

Apple Music is a global music streaming service that gives you access to more than 100 million tracks and over 30,000 playlists.

If you join a subscription for Apple Music, you can use it by simply logging in from rekordbox.

Note: If you are on an Apple Music Family plan, minors may be unable to use it due to restrictions associated with the service.

For details, check out the [Apple Music website](https://www.apple.com/apple-music/).

Click [here](https://support.apple.com/music/) to access the Apple Music Support page.

---

### What are the streaming services available in EXPORT mode? (ver. 7)

The following streaming services are available in EXPORT mode.

- Apple Music
- Beatport Streaming
- TIDAL

Tracks from streaming services cannot be exported via LINK EXPORT or to USB drives.
For details, please access [here](https://rekordbox.com/en/support/faq/streaming-7/#faq-q700037).

---

### Are there any functional restrictions when using tracks from a streaming service on rekordbox ver. 7?

The following restrictions apply when using tracks from a streaming service.

- EXPORT mode

The following streaming services are not available.
Spotify
SoundCloud
Beatsource Streaming

Tracks from all streaming services cannot be LINK EXPORTed and played.

- PERFORMANCE mode

The capture function cannot be used.
Tracks cannot be loaded to the sampler deck.
The STEMS (TRACK SEPARATION) function cannot be used on Apple Music and Spotify tracks.
The following restrictions apply on Spotify.

You cannot import tracks into [Collection].
*If you want to DJ with Spotify tracks, select Spotify in the Media Browser and load tracks from the tracklist onto your decks.
You cannot add tracks to rekordbox's [Playlists].
*Playlists created in the Spotify mobile app, desktop app, or Web Player can be accessed from the Spotify category in the Media Browser.
You cannot use Spotify in the following features:
Related Tracks, Track Suggestion, Automix
Note: Some other features may not be available.

- Common for all modes

The recording function cannot be used. This is due to copyright restrictions set by the streaming services.
Track information cannot be edited.
Music files are not backed up with the backup function.
Cannot be exported to a USB storage device or SD memory card.

---

### Can all Spotify tracks be stored offline?

No, Spotify tracks cannot be stored offline.

---

### Can I play Spotify tracks with rekordbox when I'm offline?

No, they cannot be played offline.

---

### Which countries can Spotify be used in?

For the latest information on countries and regions where Spotify is available, check out the [Spotify website](https://www.spotify.com/us/dj-integration/).

Even in regions where Spotify is available, you may not be able to use it on rekordbox (and other DJ applications).

---

### The BeatGrids and Cue points are misaligned on Apple Music tracks.

In version 7.2.8,
we have changed the specifications related to BeatGrids for Apple Music tracks.

Due to this change, BeatGrids and Cue points may become misaligned. If you notice any misalignment, please re-analyze the track and reset the Cue points.
See [here](https://rekordbox.com/en/support/faq/streaming-7/#faq-q700146) for details.

---

### How many Apple Music tracks can be played simultaneously in rekordbox?

In rekordbox for Mac/Windows, you can play up to 3 Apple Music tracks simultaneously on 3 Decks.

---

### I cannot play tracks marked with "E" on Apple Music.

To play tracks marked with "E" on Apple Music, you need to enable the following settings.
[Preferences] > [Advanced]category > [Others]tab > Apple Music > Explicit

*You may not be able to change the settings in some regions or countries.

---

### The BPM and keys of tracks on Apple Music are not shown.

To show the BPM and keys of tracks on Apple Music, you need to import the tracks to rekordbox and perform track analysis.

---

### Can all Apple Music tracks be stored offline?

No, some Apple Music tracks cannot be stored offline.

---

### Can I play Apple Music tracks with rekordbox when I'm offline?

No, they cannot be played offline.

They can only be streamed if playing them on rekordbox.

---

### Which countries can Apple Music be used in?

For the latest information on countries and regions where Apple Music is available, check out the [Apple website](https://support.apple.com/en-us/118205).

---

### Can playlists displayed on a streaming service's website or app be edited on rekordbox ver. 7?

Yes, playlists on the following streaming services can be edited.

- Sound Cloud
- Beatport
- Beatsource
- TIDAL

(As of December 2024)

---

### What is Spotify?

Spotify is a music streaming service available worldwide, offering access to over 100 million tracks.

If you subscribe to Spotify Premium, you can start using it immediately simply by logging in to rekordbox.

For more details, please visit the [Spotify website](https://open.spotify.com/).

For information on Spotify support, please check from [here](https://support.spotify.com/).

---

### I cannot add Spotify tracks to rekordbox's [Playlists].

You cannot add Spotify tracks to rekordbox's [Playlists].

Playlists created in the Spotify mobile app, desktop app, or Web Player can be accessed from the Spotify category in the Media Browser.

---

### Spotify tracks are not displayed in [Collection].

You cannot import Spotify tracks into [Collection].

Select Spotify in Media Browser and load tracks from the track list to the deck.

![](https://cdn.rekordbox.com/files/20250918192249/FAQ-Spotify-MB-286x300.png)

---

### How do I log in to TIDAL with my DJ equipment?

You can automatically log in to TIDAL by connecting a USB storage device or SD memory card created on rekordbox for CloudDirectPlay authentication to your DJ equipment and logging in to rekordbox CloudDirectPlay.

Also, with some DJ equipment, you can automatically log in to TIDAL by logging into rekordbox CloudDirectPlay using a mobile device.
Some DJ equipment allows [NFC login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100117) and [QR code login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100118). Please refer to the respective FAQs for details.

- As of rekordbox ver. 7.0.6, login to TIDAL has been changed to a Single Sign On method.
  With the Single Sign On method, if you log in to TIDAL on rekordbox in advance, you do not need to enter your login account and password for TIDAL on the DJ equipment.

---

### How do I log in to Apple Music with my DJ equipment?

You can automatically log in to Apple Music by connecting a USB storage device or SD memory card created on rekordbox for CloudDirectPlay authentication to your DJ equipment and logging in to rekordbox CloudDirectPlay.

Also, with some DJ equipment, you can automatically log in to Apple Music by logging into rekordbox CloudDirectPlay using a mobile device.
Some DJ equipment allows [NFC login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100117) and [QR code login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100118). Please refer to the respective FAQs for details.

*Apple Music uses the single sign-on method for logging in.
With the Single Sign On method, if you log in to Apple Music on rekordbox in advance, you do not need to enter your login account and password for Apple Music on the DJ equipment.

---

### Can all TIDAL tracks be stored offline?

No, some TIDAL tracks cannot be stored offline.

---

### Is there a limit to the number of days or other restrictions on the playback of streaming offline tracks?

The limitations on the playback of offline tracks for each streaming service are as follows.

- SoundCloud DJ
  Offline playback is available for 30 days from the date of offline storage. (_)
  The playability period of offline tracks will be reset to 30 days (_) by logging into SoundCloud on rekordbox with an Internet connection between the 21st and 30th day counting from the date of offline storage.
  Please note that offline tracks will be removed after the playback period of the offline track has expired.
- Beatport Streaming
  There are no restrictions.
- Beatsource Streaming
  There are no restrictions.
- TIDAL DJ Extension
  Offline playback is available for 30 days from the date of offline storage. (_)
  The playability period of offline tracks will be reset to 30 days (_) by logging into TIDAL on rekordbox with an Internet connection between the 27st and 30th day counting from the date of offline storage.
  Once the playback period of an offline track has expired, playback is no longer available.

*The duration of offline playback may vary depending on the track.
Please start rekordbox and log in to the streaming service periodically to maintain offline playback availability after 30 days.

---

### Can I play TIDAL tracks with rekordbox when I'm offline?

Yes. Select the applicable tracks on the TIDAL tracklist and store it offline to save the track to rekordbox and allow for it to be played offline.

(Ver. 7.0.4 or later)

---

### Updating rekordbox may cause you to log out from the streaming services.

Updating rekordbox may require to log in to the streaming services again.

By logging in again, you will have access to the latest functions of the streaming service.

---

### Where can I manage my TIDAL offline devices?

Access the [TIDAL website](https://tidal.com/) to manage your TIDAL offline devices.

See [here](https://support.tidal.com/hc/en-us/) for the TIDAL FAQs.

---

### Can all SoundCloud DJ tracks be downloaded offline?

Because the music labels or uploaders of some SoundCloud DJ tracks limit offline downloads, not all tracks can be downloaded offline.

---

### Can I play SoundCloud Go+/DJ tracks on rekordbox even in an offline environment?

It depends on your SoundCloud subscription plan.

- SoundCloud Go+: Tracks cannot be played offline.
- SoundCloud DJ: Tracks can be played offline if they were stored offline.

For details, please visit the [SoundCloud](https://checkout.soundcloud.com/dj) website.

---

### What do I do to use tracks on the SoundCloud Free DJ Playlist?

They will become available by clicking the [Free] button on the SoundCloud tree. Refer to the SoundCloud website for latest information on supported countries.

---

### What is Free DJ Playlists of SoundCloud?

Free DJ Playlist of SoundCloud is a playlist that can be used for free without logging in. (However, the number of available playlists is limited.)

---

### The keys of the tracks I imported to rekordbox are different from the track keys I saw listed on the streaming service.

The track keys provided by streaming services may differ from the keys of tracks that were analyzed during import to rekordbox.

To use the keys provided by streaming services as is, remove the checkmark from the KEY checkbox in the [Track Analysis Settings].

---

### Which countries can Beatsource Streaming be used in?

For the latest news on supported countries, refer to the Beatsource Streaming [website](https://www.beatsource.com/).

---

### What is Beatsource Streaming?

Beatsource Streaming is a subscription streaming service that DJs can use to access the Beatsource music catalog.

You'll need a separate subscription to use this service.

For details, refer to the Beatsource [website](https://www.beatsource.com/).

Click [here](https://support.beatsource.com/hc/en-us) for the Beatsource FAQ page.

---

### What is Beatport Streaming?

Beatport Streaming is a subscription streaming service that DJs can use to access the Beatport music catalog.

A separate signing up is required to use this service.

For details, refer to the Beatport [website](https://www.beatport.com/get-link).

Click [here](https://support.beatport.com/hc) for the Beatport FAQ page.

---

### Which countries can Beatport Streaming be used in?

For the latest news on supported countries, refer to the Beatport [website](https://support.beatport.com/hc/en-us/articles/5586662728980-In-what-countries-is-Beatport-Streaming-available-).

---

### Can I play Beatport Streaming tracks on rekordbox even in an offline environment?

Yes. Select your desired tracks from the track list in the Beatport category of the tree view and store them to the rekordbox by choosing [Store Offline] to play them in an offline environment.

---

### In what countries can SoundCloud Go+ be used?

For the latest news on supported countries, refer to the SoundCloud [website](https://help.soundcloud.com/hc/en-us/articles/360051736074).

---

### The streaming service tracks I added to the collaborative playlist are not being displayed on the shared members' collaborative playlists.

The shared members need to log in to the streaming service for the streaming service tracks to be displayed.

The shared members must be subscribed as well as logged in to the same streaming service.

---

## Related Documents

- [features/overview.md](../features/overview.md) (streaming)
- [features/whats-new-v7.md](../features/whats-new-v7.md) (streaming)
- [guides/streaming-services.md](../guides/streaming-services.md) (streaming)
