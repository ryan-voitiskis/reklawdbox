---
id: faq-hardware-compatibility
title: "FAQ: Hardware Compatibility"
type: faq
source:
  file: "rekordbox7-faq.md"
  url: "https://rekordbox.com/en/support/faq/rekordbox7/"
  version: "7.x"
topics: [color, compatibility, dvs, equipment, export, file-formats, onelibrary, usb, waveform]
modes: [common]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

### What is the color of the waveform (BLUE/RGB/3Band) that can be displayed on the hardware display?

The color of the waveform that can be displayed depends on the model.

**BLUE/RGB/3Band**

- CDJ-3000X
- CDJ-3000
- DDJ-FLX10
- DDJ-REV7
- OPUS-QUAD
- OMNIS-DUO
- XDJ-RX3

**BLUE/RGB**

- CDJ-TOUR1
- CDJ-2000NXS2
- XDJ-1000MK2
- XDJ-XZ
- XDJ-RX2
- XDJ-RR
- DDJ-1000　*1
- DDJ-800　*1
- DJM-S11　*1

**BLUE**

- CDJ-2000NXS
- CDJ-900NXS
- XDJ-1000
- XDJ-700
- XDJ-RX

*1 For this DDJ/DJM series, the color of the waveform can be changed in the [Preferences] of rekordbox.When set to 3Band, the color of the waveform is displayed in BLUE.

(List correct as of January 2026)

---

### Which music file formats are supported by the DJ equipment?

Supported music file formats vary depending on DJ equipment.

Please check the table below and use music file formats supported by your DJ equipment.

<!-- dprint-ignore -->
|                                                                 | **88.2/96KHz** | **44.1/48KHz**     |         |          |         |          |                    |         |          |         |         |
| --------------------------------------------------------------- | -------------- | ------------------ | ------- | -------- | ------- | -------- | ------------------ | ------- | -------- | ------- | ------- |
|                                                                 | **FLAC**       | **Apple Lossless** | **WAV** | **AIFF** | **AAC** | **FLAC** | **Apple Lossless** | **WAV** | **AIFF** | **mp3** | **AAC** |
| CDJ-3000X  CDJ-3000  CDJ-TOUR1  CDJ-2000NXS2  OPUS-QUAD  XDJ-AZ | ○              | ○                  | ○       | ○        | ✕       | ○        | ○                  | ○       | ○        | ○       | ○       |
| XDJ-1000MK2  OMNIS-DUO                                          | ✕              | ✕                  | ✕       | ✕        | ✕       | ○        | ○                  | ○       | ○        | ○       | ○       |
| XDJ-XZ  XDJ-RX3                                                 | ✕              | ✕                  | ✕       | ✕        | ✕       | ○        | ✕                  | ○       | ○        | ○       | ○       |
| other                                                           | ✕              | ✕                  | ✕       | ✕        | ✕       | ✕        | ✕                  | ○       | ○        | ○       | ○       |

---

### What is [Use FX1 and FX2 as FX SEND/RETURN] of [BEAT FX] in PERFORMANCE mode? (v7 [Preferences] > [Controller] category > [Effect] tab)

This is shown only when an FX SEND/RETURN compatible DJ equipment*1 is connected and selected as the audio device.

When you select this option, FX (e.g. ECHO or REVERB) tails can be heard even after pulling the channel fader of the mixer all the way down.
Please note that FX1 and FX2 must be set to the same mixer channel in this case.
This is because both of the FX1 and FX2 are routed to [FX SEND/FX RETURN] audio routing of the mixer.

When you unselect this option, the FX1 and FX2 are independently routed to the internal audio routing of rekordbox.
Therefore you can set the FX1 and FX2 to separate mixer channels.
However, FX tails can NOT be heard even after pulling the channel fader of the mixer all the way down.

*1 FX SEND/RETURN compatible DJ equipment:

DJM-V10
DJM-V5
DJM-TOUR1
DJM-A9
DJM-900NXS2
DJM-750MK2
DJM-450
DJM-250MK2
euphonia
XDJ-XZ

(As of January 2026)

---

### What DJ equipment can automatically select decks for lighting in PERFORMANCE mode? (ver. 7)

Connect the DJ equipment and PC/Mac with a USB cable, and check [Preferences] > [Audio]category > [Input/Output]tab > [Mixer Mode] (in PERFORMANCE mode).

- If your DJ equipment supports internal mixer mode, automatic deck selection is available when [Internal] is selected.
- If your DJ equipment supports external mixer mode, automatic deck selection is available only on the following models when [External] is selected.

DJM-V10*
DJM-V5
DJM-TOUR1
DJM-A9
DJM-900NXS2
DJM-750MK2
DJM-450
DJM-250MK2
DJM-S11
DJM-S9
DJM-S7
euphonia
DDJ-SZ
OPUS-QUAD
XDJ-XZ

*NOTE: The DJM-V10-LF does not support this automatic deck selection feature. If the auto mode does not work as expected, please select the deck manually.

(As of Jan 2026)

---

### Which DJ equipment is compatible with Cloud Analysis (ver. 7)?

Cloud Analysis is compatible with the following DJ equipment.

- CDJ-3000X
- CDJ-3000
- OPUS-QUAD
- OMNIS-DUO
- XDJ-AZ

(As of September 2025)

---

### Which of the DJ equipment are compatible with rekordbox CloudDirectPlay (ver. 7)?

You can use rekordbox CloudDirectPlay with the following compatible DJ equipment:

- CDJ-3000X
- CDJ-3000
- OPUS-QUAD
- OMNIS-DUO
- XDJ-AZ

(As of September 2025)

---

### What do I need to use Cloud Analysis on DJ equipment (ver. 7)?

To use Cloud Analysis on DJ equipment, you must prepare the following environment and settings.

- Network connection environment
- rekordbox for Mac/Windows ver. 7.
- Authentication device (USB storage device or SD memory card) *

*Some DJ equipment allows [NFC login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100117) and [QR code login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100118). Please refer to the respective FAQs for details.

---

### What do I need to start using rekordbox CloudDirectPlay (ver. 7)?

You will need the following listed below:

- An authentication device (USB storage device or SD memory card) . *1
- A subscription for the Creative, Professional plan, or Cloud Option. *2
- rekordbox for Mac / Windows.
- Cloud Library Sync turned on.
- A Dropbox account, logged in.

Refer to the "rekordbox CloudDirectPlay Operation Guide" on the [Manuals](https://rekordbox.com/en/download/#manual) page for how to use rekordbox CloudDirectPlay.

*1 Some DJ equipment allows [NFC login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100117) and [QR code login](https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100118). Please refer to the respective FAQs for details.
*2 With Core plan and Free plan, CloudDirectPlay is limited to up to 20 tracks in [Trial playlist - Cloud Library Sync]. With Hardware Unlock or owner registration, you can also use Cloud Export for up to 1000 songs.

---

### Is there a way to confirm what can be browsed on the CDJ-3000X for a USB storage device exported with rekordbox ver. 7?

You can confirm the contents on the rekordbox ver. 7 screen.

- You can confirm in the same way when using the XDJ-AZ, OPUS-QUAD, or OMNIS-DUO.

Procedure for checking USB storage device content browseable on the CDJ-3000X

1. Update rekordbox ver. 7 to the latest version.
   In rekordbox, go to [Help] menu > [rekordbox update manager] to perform the update.
2. Launch rekordbox and connect a USB storage device into your PC/Mac
3. From the [Devices] of the Media Browser, select your USB storage device name > [OneLibrary] to check the contents.
   If the displayed tracks or playlists are missing, you must export OneLibrary to the USB storage device again. For details, please refer to the [FAQ](https://rekordbox.com/en/support/faq/onelibrary-7/#faq-q700038).

---

### I exported tracks and playlists from rekordbox ver. 7 to a USB storage device, but when I connect the device into the CDJ-3000X, I cannot browse the tracks or playlists. The device works fine on other players.

The CDJ-3000X can browse tracks and playlists on a USB storage device only if OneLibrary (formerly Device Library Plus) has been exported to that device.
Follow the steps below to export OneLibrary.

Note: If you are using XDJ-AZ, OPUS-QUAD, or OMNIS-DUO, export OneLibrary using the same procedure as for the CDJ-3000X.

OneLibrary Export Procedure

1. Update rekordbox ver. 7 to the latest version
   In rekordbox, go to [Help] menu > [rekordbox update manager] to perform the update.
2. Launch rekordbox and connect a USB storage device into your PC/Mac
3. In the Media Browser, go to [Devices] > your USB storage device name, right-click [OneLibrary], and select [Convert from Device Library].
   ![](https://cdn.rekordbox.com/files/20251224094129/Convert-device-library-en.png)

This operation exports a OneLibrary with the same content as the existing Device Library to the USB storage device.

After that, you will be able to browse tracks and playlists on the CDJ-3000X just as you can on other players.

From then on, whenever you export tracks or playlists to a USB storage device, the files will be automatically added to both the existing Device Library and the OneLibrary.

Caution: If you convert when OneLibrary already exists on the USB storage device, it will be overwritten. As a result, any playlists or playback histories stored only in OneLibrary will be lost.
For more details, please refer to the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf)".

---

### What is OneLibrary?

This is a new library format created by rekordbox on the USB storage device (or SD card).

DJ equipment supports traditional Device Library formats or the new OneLibrary formats.

- See the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf#page=2)" for more details.
- See [here](https://rekordbox.com/en/support/usb-export/) for the supported DJ equipment.

---

### Which DJ equipment supports OneLibrary or the traditional Device Library is unclear.

See [here](https://rekordbox.com/en/support/usb-export/) for the supported DJ equipment.

---

### Can I convert USB libraries I've been using into OneLibrary?

Yes, you can.

You can convert the traditional Device Library in the USB storage device to the OneLibrary format with rekordbox for Mac/Windows.

- If the following dialog box appears when connecting a USB storage device, select [OK] to convert.
  ![](https://cdn.rekordbox.com/files/20251002150123/OneLibrary001_En-300x171.png)
- Select the device from the media browser, click [Convert from Device Library] from the OneLibrary context menu and follow the instructions.
  ![](https://cdn.rekordbox.com/files/20251002150426/OneLibrary003_En-300x122.png)

See the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf#page=2)" for more details.

---

### The playlist and Histories of the traditional Device Library supporting equipment are not displayed on the OneLibrary supporting device.

There may be differences between the playlist and Histories of the 2 libraries.

You can synchronize 2 libraries with rekordbox for Mac/Windows.

See the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf#page=4)" for more details.

---

### I've deleted playlists and tracks from Device Library, but they are not deleted from OneLibrary. How can I delete them from both libraries?

Deleting a playlist or track is reflected only on the library for which the control is performed. Perform deletion controls on both of the libraries.

![](https://cdn.rekordbox.com/files/20251022074026/QA_illust_720_4-onelibrary.png)

---

### The playlist and Histories of the OneLibrary supporting equipment are not displayed on the traditional Device Library supporting device.

There may be differences between the playlist and Histories of the 2 libraries.

You can synchronize 2 libraries with rekordbox for Mac/Windows.

See the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf#page=4)" for more details.

---

### Are there changes in use methods for LINK EXPORT since a new format of OneLibrary was added?

No, there aren't.

In the same way as before, you have access to LINK EXPORT.

---

### If there are differences in the playlists and Histories of the OneLibrary and the traditional Device Library, how can I synchronize them?

There may be differences between the playlist and Histories of the 2 libraries.

You can synchronize 2 libraries with rekordbox for Mac/Windows.

See the "[USB Export Guide](https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf#page=4)" for more details.

---

### The DJ equipment I am using is not listed in the compatible DJ units on rekordbox.com.

Some products that have been discontinued for a long time may no longer be supported by rekordbox. The following DJ equipment is not guaranteed to work.

Examples of equipment no longer supported by rekordbox

- DDJ-RB
- DDJ-RR
- DDJ-RX
- DDJ-RZ
- DDJ-XP1
- XDJ-R1

*DDJ-RZX is not compatible with rekordbox ver. 7.
Please use rekordbox ver. 6. You can download rekordbox ver. 6 from the [archive](https://rekordbox.com/en/support/faq/v6/#faq-q600141).

For the latest information, please refer [Compatible DJ units](https://rekordbox.com/en/support/link/) page.

---

### Exported SD memory cards or USB storage devices could not be recognized by DJ equipment, and tracks are not displayed.

Your device may be formatted in a file system that is not supported by the DJ equipment.

If the file system is exFAT, only [these DJ equipment](https://rekordbox.com/en/support/faq/v7/#faq-q700010) are available.

If you intend to use other DJ equipment, go [here](https://rekordbox.com/en/support/faq/v7/#faq-q600157), and format to a file system other than exFAT for use.

---

### Which DJ equipment supports exFAT?

The following DJ equipment supports exFAT.

- CDJ-3000X
- CDJ-3000
- OPUS-QUAD
- OMNIS-DUO
- XDJ-XZ
- XDJ-RX3

As of September 2025

---

### Which file system should I format to in order to use SD memory cards and USB storage devices?

File systems supported by rekordbox are the following.

![](https://cdn.rekordbox.com/files/20220323175522/filesystem_EN22.png)

*Go [here](https://rekordbox.com/en/support/faq/v7/#faq-q700010) for DJ equipment that support exFAT.

Refer to the Pioneer DJ support page for SD memory cards and USB storage devices that are available on DJ equipment.

---

### Play histories in a USB storage device connected in PERFORMANCE mode are not imported. The context menu for [import History] is grayed out.

Importing play histories is limited to EXPORT mode.
Switch to EXPORT mode to use this function.

---

### Even if [Import the play history automatically] is set to ON, there are some play histories that are not imported.

Even if [Import the play history automatically] is set to ON, the same name and contents as the previously imported play history will not be imported automatically.

You can manually import the play histories in the following ways.
Select the [Display Devices] icon in the Media Browser, select the play history you wish to import from the [Histories] folder in the USB storage device (or SD memory card), right-click and select [import History] from the context menu.

---

### After importing the play histories in a USB storage device, the play histories are deleted from the device.

The default setting is to delete the play histories from the device when importing the play histories in the USB storage device (or SD memory card).

By changing the following settings, you can keep the play histories on your device.
Turn OFF [Prefernces] > [Devices] category > [Delete from the device after importing the play history].

---

### Can Key Shift and Keyboard of PAD MODE be used on LIST Display during DDJ-GRV6 connection?

Key Shift and Keyboard of PAD MODE can be used on PAD Display during DDJ-GRV6 connection. (Does not support LIST Display)

Set [PAD mode] to [Auto] or [Customize] > [PAD Display].
([Preferences] > [View]category > [Layout]tab)

---

### There is nothing displayed on the DDJ-RZX hardware screen.

DDJ-RZX is not supported on rekordbox ver. 7.

---

### I turned off the SYNC button in the DVS RELATIVE mode and moved the Tempo Slider on the turntable (CDJ/XDJ) to the center position (+/-0%) , but the BPM of the track on the DECK in the RELATIVE mode is not the original BPM.

Hover the mouse over the platter to show [RESET]*. Click [RESET] to return to the original BPM.

![](https://cdn.rekordbox.com/files/20200214140322/FAQ315-1.png)

*When selecting [2Deck Horizontal] or [4Deck Horizontal], [R] is shown instead of [RESET].

![](https://cdn.rekordbox.com/files/20200214140402/FAQ315-2.png)

The following procedures will solve the problem.

1. Turn off the SYNC button on the DECK in the RELATIVE mode.
2. Move the Tempo Slider on the turntable (CDJ/XDJ) to +/-0%.
3. Switch the DVS mode to INTERNAL, and move the Tempo Slider on the virtual DECK to +/-0% using a mouse. (Double-click the Tempo Slider knob, and the slider will move to the +/-0% position.)
4. Switch the DVS mode to RELATIVE.

---

### BeatGrid is not synced even if the SYNC button is turned on in the DVS RELATIVE mode.

In the RELATIVE mode, BeatGrid is not synced. Only BPM is synced.

---

### Can I use Control Vinyl or CD of other manufacturers?

No. We do not support any control vinyl, CD nor control signal WAV data of other manufacturers.

If you use them, they won't work properly, for example, the marker on the vinyl and the cue point and the playhead positions may not match.

---

### How can I use the DVS feature on multi-players (CDJ, XDJ)?

To use the DVS feature on multi-players (CDJ, XDJ), the control signal exclusive for DVS is needed.

Please download our control signal WAV file

[rekordbox_Control_Signal.zip](https://cdn.rekordbox.com/files/20200213120750/rekordbox_Control_Signal.zip)

and burn it on CD-R or save it in USB memory, etc.

---

### Can STEMS function be controlled by DJ equipment?

Yes, it can be controlled by DJ equipment on which STEMS function is mounted.

In addition, you can use each function by assigning it to the hardware on MIDI LEARN.

However, it is necessary to subscribe to a supported Plan or connect a HardwareUnlock device.

---

### Can GROOVE CIRCUIT be controlled through DJ equipment?

GROOVE CIRCUIT can be controlled using the following DJ equipment.

- DDJ-GRV6

(As of October 2024)

Each function can be used through mapping with the MIDI LEARN function on other DJ equipment. (The MIDI LEARN function requires a [Core plan subscription](https://rekordbox.com/en/support/faq/stems-7/#faq-q700019) or higher, or connection of [Hardware Unlock devices](https://rekordbox.com/en/support/faq/v7/#faq-q700001).)
See ["MIDI LEARN Operation Guide"](https://rekordbox.com/en/download/#manual) for mapping methods using the MIDI LEARN function.

---

### Can MIX POINT LINK function be controlled by DJ equipment?

Yes, it can be controlled by DJ equipment on which MIX POINT LINK function is mounted.

In addition, you can use each function by assigning it to the hardware on MIDI LEARN.

However, it is necessary to subscribe to a supported Plan.

---

### Can 4 Stems be operated on DJ equipment?

You can map each function using the MIDI LEARN feature. (The MIDI LEARN feature requires a [Core plan subscription](https://rekordbox.com/en/support/faq/stems-7/#faq-q700019) or higher, or connection of [Hardware Unlock devices](https://rekordbox.com/en/support/faq/v7/#faq-q700001).)

For information on how to map using the MIDI LEARN feature, please refer to the [MIDI LEARN Operation Guide](https://rekordbox.com/en/support/faq/stems-7/#faq-q700019).

For DJ controllers that support PAD EDITOR, you can map each function using the PAD EDITOR feature.

For information on how to map using the PAD EDITOR feature, please refer to the [PAD EDITOR guide](https://rekordbox.com/en/download/#manual).

---

### What are the DMX interfaces that are supported by the Lighting function?

The DMX interfaces that currently supported by the Lighting function are as follows.

- RB-DMX1
- DDJ-FLX10
- ENTTEC Open DMX
- ENTTEC DMX USB Pro
- ENTTEC DMX USB Pro Mk2

---

## Related Documents

- [faq/usb-and-devices.md](usb-and-devices.md) (equipment, export, usb)
- [guides/dvs-setup.md](../guides/dvs-setup.md) (compatibility, dvs, equipment)
- [guides/performance-mode-connection.md](../guides/performance-mode-connection.md) (compatibility, dvs, equipment)
- [manual/14-export-playing.md](../manual/14-export-playing.md) (export, usb, waveform)
- [manual/19-performance-preparing.md](../manual/19-performance-preparing.md) (equipment, export, waveform)
- [faq/cloud-and-sync.md](cloud-and-sync.md) (export, onelibrary)
- [guides/device-library-backup.md](../guides/device-library-backup.md) (export, usb)
- [guides/usb-export.md](../guides/usb-export.md) (export, usb)
