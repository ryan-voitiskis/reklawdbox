---
id: export-lan
title: "Using the LAN connection"
type: manual
source:
  file: "rekordbox7.2.8_manual_EN.pdf"
  pages: "109-113"
  version: "7.2.8"
topics: [browsing, connection, devices, export, mixing, pro-dj-link]
modes: [export]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

# Using the LAN connection

If you connect a computer to a DJ player by a LAN cable or wireless LAN, you can directly load rekordbox tracks and data, and use them on the DJ player. (page 111)

- For transferring tracks to DJ equipment, it is not necessary to export them to USB storage devices.
- You can use various rekordbox track-selecting features (e.g. track browsing) during your DJ performance.
- On rekordbox, you can check the play histories of DJ players (page 102). A play history by PRO DJ LINK is created in the year's folder as [LINK HISTORY yyyy-mm-dd].

When your computer is connected to a PRO DJ LINK-compatible DJ mixer by a LAN cable, you can monitor tracks in rekordbox through headphones connected to the DJ mixer. (page 110)

## Establishing the link

Depending on your computer's security software and operating system settings, it may not be possible to establish links with DJ equipment. In this case, it is necessary to clear settings for any programs and communications ports that are blocked.

- For the link status display, see "Link status panel" (page 81).

[Screenshot: Link status panel showing 4 DJ players connected with track names, MASTER SYNC indicators, and MIXER label]

1. Connect the computer and DJ equipment to the network.
   When detecting DJ equipment connected to the network, the LINK icon is displayed at the bottom left of the browser panel.

2. Click the LINK icon.
   Communication with DJ equipment connected by a LAN cable or wireless LAN is enabled.
   The link status panel (icons of connected DJ equipment) is displayed at the bottom of the browser panel, and the LINK icon is displayed.

**Hint**

- For connection instructions, and the unit number of DJ players that can be connected, refer to the Instruction Manual for the DJ equipment.
- When connected using a switching hub or a PRO DJ LINK-compatible DJ mixer, rekordbox music files and data can be shared with 4 DJ players (6 DJ players for CDJ-3000 only).
- It may take time for the network address to be acquired automatically, depending on the communications environment.
- When the LINK icon is displayed on the left side of the link status panel, there are two computers connected, one of which has rekordbox installed.
- When the wireless icon is displayed on the left side of the link status panel, the computer is connected to the network by a wireless LAN.
- When [MIDI/HID] is displayed on the right side of the DJ equipment icon, DJ equipment is communicating with another computer by USB control (MIDI or HID).

### To change the displaying order of the DJ equipment icons in the link status panel

The order in which DJ equipment icons display in the link status panel can be changed by dragging them left and right.

### To exit the link

Click the LINK icon to cancel the communication with DJ equipment connected by a LAN cable or wireless LAN.

## Monitoring tracks through headphones connected to the DJ mixer

To monitor rekordbox tracks through headphones connected to the DJ mixer, open the [Preferences] window > [DJ system] category > [Others] tab, select [Use "LINK MONITOR" of the DJ Mixer], and then start playback.

- For instructions on the DJ mixer, refer to the Instruction Manual for the DJ mixer.

**Hint**

- The click sound of the waveform on the [Preview] column or [Artwork] column is also monitored with headphones from the DJ mixer.

## Using a DJ player

Drag a track from a track list in the browser panel to the DJ equipment icon in the link status panel. The track is loaded onto the DJ player, and playback starts.

**Note**

- When the [EJECT/LOAD LOCK] function of a DJ player is active, tracks cannot be loaded until playback on the DJ equipment pauses.
- Tracks in [Devices] cannot be loaded onto a DJ player.

### To use the quantize function on a DJ player or DJ mixer

If you have detected and adjusted beat grids of tracks using rekordbox, you can use them with the quantize function on performing cue operations and playing loops at the DJ player. Furthermore, if a DJ player and a DJ mixer are connected by a LAN cable, you can use the quantize function for special effects (FX).

- For instructions on using the quantize function on a DJ player or DJ mixer, refer to the Instruction Manual for DJ equipment.

### To use the beat sync function between DJ players or all-in-one DJ system for DJ performance

If you have detected and adjusted beat grids of tracks using rekordbox, you can synchronize tempos (BPM) and beats of DJ players connected via PRO DJ LINK. You can also synchronize them of the left and right controller decks.

- For instructions on using the beat sync function on a DJ player, refer to the Instruction Manual of the DJ player.

**Hint**

- You can synchronize tempos (BPM) and beats of multiple DJ equipment by specifying tempos (BPM) on rekordbox.

### To use Hot Cues on DJ equipment

The Hot Cue ([A] - [H]) information of music files can be called and used on DJ equipment.

- For instructions on using Hot Cues on DJ equipment, refer to the Instruction Manual of DJ equipment.

**Hint**

- When [Auto Load Hot Cue] is enabled and such tracks are loaded onto a DJ player, Hot Cues saved in tracks are automatically loaded.
- The number of Hot Cues depends on the DJ player.

### To load the Hot Cue Bank Lists onto a DJ player

Drag the required Hot Cue Bank List from the [Hot Cue Bank Lists] to the DJ equipment icon in the link status panel. The Hot Cue Banks stored in the Hot Cue Bank Lists are loaded into the Hot Cues of the DJ player.

**Hint**

- The number of available Hot Cues depends on the DJ player.

### To share tracks by using Tag List

Tag List is a list allowing you to perform real-time browsing from each DJ player displayed in the link status panel.
When tracks are added from rekordbox to Tag List, the tracks on Tag List can be loaded onto the DJ player and played by operating the DJ player.

1. Open the [Preferences] window > [View] category > [Layout] tab, and check the [Playlist Palette] checkbox of [Browser panel].

2. Click the playlist palette icon in the browser panel to display the playlist palette.

3. Click [TAG] above the tree view, and then click the tag list expand icon on the right side of [TAG].

4. Drag a track from [Collection] in the browser panel to [Tag List].
   The tracks are added to [Tag List].

**Hint**

- Tracks can also be added by right-clicking a track and selecting [Add to Tag List].
- Tracks and playlists can also be added by dragging them from [Playlists] or [iTunes].
- Up to 100 files can be added.

**Change the order of tracks on Tag List**

1. Click the heading of the column displaying the track order.
   Each time you click, the arrangement switches between ascending and descending order.

2. Drag a track to change its position in the list.

**Note**

- If tracks are sorted by any column header other than track order, you cannot change the track order by dragging a track.

**Play tracks on a DJ player by using Tag List**

By operating the DJ player, tracks on Tag List can be loaded and played on each DJ player, and tag lists actually used during performances can be saved as rekordbox playlists. For instructions on accessing tag lists from the DJ player, refer to the Instruction Manual of the DJ player.

## Related Documents

- [manual/19-performance-preparing.md](19-performance-preparing.md) (browsing, devices, export, mixing)
- [faq/usb-and-devices.md](../faq/usb-and-devices.md) (devices, export, pro-dj-link)
- [guides/pro-dj-link-setup.md](../guides/pro-dj-link-setup.md) (connection, devices, pro-dj-link)
- [manual/10-mobile-devices.md](10-mobile-devices.md) (connection, devices, export)
- [manual/13-export-pro-dj-link.md](13-export-pro-dj-link.md) (connection, export, pro-dj-link)
- [guides/cloud-direct-play.md](../guides/cloud-direct-play.md) (devices, pro-dj-link)
- [guides/device-library-backup.md](../guides/device-library-backup.md) (devices, export)
- [guides/dvs-setup.md](../guides/dvs-setup.md) (connection, mixing)
