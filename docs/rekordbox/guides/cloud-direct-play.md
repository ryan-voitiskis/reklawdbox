---
id: cloud-direct-play
title: "CloudDirectPlay Operation Guide"
type: guide
source:
  file: "rekordbox7.2.2_CloudDirectPlay_EN.pdf"
  pages: "1-24"
  version: "7.2.2"
topics: [cloud, devices, playback, pro-dj-link, usb]
modes: [common, export]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

# rekordbox CloudDirectPlay Operation Guide

## About this manual

This manual explains about rekordbox CloudDirectPlay. Read "rekordbox Introduction" and "Instruction Manual" for instructions on rekordbox in general.
rekordbox.com/en/download/#manual

- In this manual, the name of buttons and menus displayed on rekordbox are indicated with brackets (e.g. [BPM], [Collection] window).
- Please note that depending on the operating system version, web browser settings, etc., operation may differ from the procedures described in this manual.
- Please note that the language on the rekordbox screen described in this manual may differ from the language on your screen.
- Please note that the specifications, design, etc. of rekordbox may be modified without notice and may differ from the descriptions in this manual.

# Introduction

## About CloudDirectPlay

When using CloudDirectPlay, you can download a music file uploaded to the cloud storage to supported DJ equipment, and then you can play it.

To upload a music file to cloud storage, use the Cloud Library Sync function. By using a cloud storage for uploading music files, you can download and play them on your PC/Mac, mobile devices, or compatible DJ equipment.
For Cloud Library Sync, refer to Cloud Library Sync Operation Guide.
rekordbox.com/manual/
For an overview, refer to the Features page on rekordbox.com.
For CloudDirectPlay compatible DJ equipment, refer to the FAQ.
rekordbox.com/en/support/faq/clouddirect-7/#faq-q700150

### rekordbox version

To use CloudDirectPlay, install the latest version of rekordbox.

### Subscription

CloudDirectPlay is available with any plan including with Free Plan. Free and Core plans can use up to 20 tracks in the [Trial playlist - Cloud Library Sync]. For details about plans, refer to the Plan page on rekordbox.com.
rekordbox.com/en/plan/

### Cloud storage service

CloudDirectPlay uses a cloud storage service used for Cloud Library Sync.
CloudDirectPlay supports Dropbox and Google Drive. (As of March 2025)
For more details, refer to Cloud Library Sync Operation Guide.
rekordbox.com/manual/

### Internet speed

The time required for library synchronization and music file download varies greatly depending on the strength of your Internet connection. With CloudDirectPlay an Internet speed of 20 Mbps or higher is recommended for download.

### Individual use

With CloudDirectPlay individuals can use the same library on multiple PC/Mac, mobile devices, and DJ equipment but multiple people cannot share the same library.

# rekordbox (Mac/Windows)

Use CloudDirectPlay with rekordbox for Mac/Windows.

## Using CloudDirectPlay

1. On rekordbox for Mac/Windows, open the [Preferences] window and click the [Cloud] category.

2. Click the [Library Sync] tab.
   Check that [Sync library to another device] of [Cloud Library Sync] is turned on.

3. Click the [DJ equipment linkage] tab.

4. Turn on [Using device authentication] and [Use rekordbox CloudDirectPlay] of [Cloud Analysis / rekordbox CloudDirectPlay].

[Screenshot: Preferences window showing Cloud category, DJ equipment linkage tab. Cloud Analysis / rekordbox CloudDirectPlay section with toggles for "Using device authentication" and "Use rekordbox CloudDirectPlay" both enabled. Log Out Time section shows automatic logout time options for DJ equipment during QR code or NFC login: 240min, 120min, 60min, 30min.]

On the cloud server, the library conversion will begin.

**Note**

- The library conversion may take time according to the number of music files in the library.
- If a library conversion error occurs, [Use rekordbox CloudDirectPlay] will be automatically turned off. If the library conversion error occurs repeatedly, please contact the support from rekordbox.com.
- When [Use rekordbox CloudDirectPlay] is turned off, the authentication of all devices in the same account will be canceled.
- When [Use rekordbox CloudDirectPlay] is tuned off and then back on, the library conversion will begin again.
- If you log in to rekordbox CloudDirectPlay using QR code or NFC login, you will be automatically logged out of rekordbox CloudDirectPlay after the time set in [Automatic logout time for DJ equipment during QR code or NFC login.] elapses.

## About an authenticated device for CloudDirectPlay

To download and play music files on DJ equipment supporting CloudDirectPlay, an authenticated device (USB storage device or SD memory card) is required. Connect the device to your PC/Mac and click the authentication button to use the device as an authenticated device for CloudDirecPlay.

### To authenticate the device

1. Click the authentication button on the right of the device name in [Devices] of Media Browser.

[Screenshot: Devices section in Media Browser showing a device with play, eject, and Auth (authentication) buttons.]

When the device has been authenticated, the authentication button will change to (authenticated icon).

### To cancel the device authentication

Click (authenticated icon) again to cancel the authentication.

**Note**

- If you have multiple accounts: Switch the account to the account that authenticated the device, then cancel the authentication.
- If the account belongs to someone else: Cancel the authentication with connecting the device to the PC/Mac of that account or cancel the authentication on the website from the PC/Mac of that account.

## Canceling the device authentication on the website

You can cancel the authentication on the website, such as if you lose your device.

1. On rekordbox for Mac/Windows, open the [Preferences] window and click the [Cloud] category.

2. Click the [DJ equipment linkage] tab.

3. Turn on [If you've lost an authenticated device].

[Screenshot: Preferences window showing Cloud category, DJ equipment linkage tab. Cloud Analysis / rekordbox CloudDirectPlay section with "Using device authentication" and "Use rekordbox CloudDirectPlay" toggles, and a highlighted link "If you've lost an authenticated device" at the bottom.]

The website appears on the browser. Cancel the authorization on the website.

**Note**

- You cannot cancel the authorization with another account. It is necessary to switch the account before the cancelation.

## Preparing for using CloudDirectPlay Filter

1. Click the [CloudDirectPlay Filter] icon in Media Browser.
   When using it for the first time, if the necessary settings are not enabled, the (?) button will be displayed.
   Click the (?) button to display the guide for the settings.

[Screenshot: Media Browser sidebar showing CloudDirectPlay Filter icon selected at the top, with various other navigation icons below it.]

2. Enable [Step1] and [Step2] according to the on-screen display.

[Screenshot: Dialog titled "How do I use Cloud Export/CloudDirectPlay Filter?" showing two steps side by side. Step1: Enable Cloud Library Sync - shows Cloud Library Sync toggle for "Sync library to another device" with an "Open Preferences" button. Step2: Enable CloudDirectPlay - shows Cloud Analysis / rekordbox CloudDirectPlay toggles for "Using device authentication" and "Use rekordbox CloudDirectPlay" with an "Open Preferences" button. A "Close" button at the bottom.]

The CloudDirectPlay Filter will be available.

**Note**

- To use the CloudDirectPlay Filter, the Cloud Option or a Creative/Professional Plan is required.

## Setting display of playlists using CloudDirectPlay Filter

1. Click the (settings) button of [CloudDirectPlay Filter].

[Screenshot: CloudDirectPlay Filter header bar with settings button highlighted.]

The [CloudDirectPlay Filter] window will appear, and you will be able to select playlists.
On the [CloudDirectPlay Filter] window, playlists in rekordbox are displayed in the tree view.

2. Check the playlist or folder to use with CloudDirectPlay on the [CloudDirectPlay Filter] window.

[Screenshot: CloudDirectPlay Filter window showing a tree view of playlists on the left side with checkboxes. Playlists include folders like "Car Playlist", "STANDARD", "House", "Techno mix", "Drum'n'bass", "Imported Playlists" with subfolders. On the right side, a "Hints" section explains "How do you use this window? Remove the check from playlists you won't be using when DJing. (They will not be displayed when using CloudDirectPlay on DJ equipment)". Navigation dots at the bottom. Legend shows cloud icon for "Upload tracks in the playlist to the cloud storage automatically" and folder icon for "A playlist with no tracks uploaded to the cloud". OK and CANCEL buttons at the bottom.]

By turning off the checkbox of each playlist or folder, you can hide unnecessary playlists or folders when using CloudDirectPlay.

When you turn on the [Set the selected playlist to the Auto Upload setting] checkbox, the Auto Upload function will automatically upload the selected playlist to the cloud storage, and you can use music files included in the playlist on CloudDirectPlay.

3. Click the [OK] button.
   The playlist selection window will close, and the settings will be applied.

**Hint**

- For [Auto Upload], refer to the Cloud Library Sync Operation Guide.
  rekordbox.com/en/download/#manual
- If none of the music files in a playlist have been uploaded to the cloud storage, that playlist will not be displayed on CloudDirectPlay compatible equipment.

# rekordbox (iOS/Android)

Use CloudDirectPlay with rekordbox for iOS/Android on mobile device.
This section explains about rekordbox iOS/Android version 4.5.4 or later.
If you use rekordbox iOS/Android earlier than version 4.5.4, update to the latest version.

## Using CloudDirectPlay

1. On rekordbox for iOS/Android, tap (account icon) in the upper-right of the screen.

[Screenshot: rekordbox iOS/Android Browse screen showing menu items: Collection, Playlists, Histories, Related Tracks. Account and settings icons in the upper-right corner.]

The login screen will appear.

2. Enter your email address and password, and log in.

3. Tap (account icon) in the upper-right of the screen.
   The [Account Information] screen will appear.

4. Tap [Activate] and [Cloud Library Sync] to turn them on.

[Screenshot: Account Information screen on iOS/Android showing email address field, Subscription Plan showing "Creative Plan" valid until 01 10, 2026, with toggles for "Activate" and "Cloud Library Sync" both enabled. Description text reads "Automatically syncs the rekordbox library information with the device logged in with the same AlphaTheta account using the cloud service."]

5. Log in to Dropbox or Google Drive.

6. On the [Account Information] screen, tap [rekordbox CloudDirectPlay] to turn it on.

[Screenshot: Account Information screen on iOS/Android showing Creative Plan valid until 01 10, 2026, Activate and Cloud Library Sync toggles enabled. Cloud Library Sync section with "Cloud Storage Services" option. rekordbox CloudDirectPlay section with toggle for "rekordbox CloudDirectPlay" enabled. Description text reads "A function that enables you to select tracks from the cloud rekordbox library and play them on compatible DJ equipment."]

# Compatible DJ equipment

## Using CloudDirectPlay compatible DJ equipment

To use CloudDirectPlay on DJ equipment, connect the PRO DJ LINK network to the Internet and use the device authenticated on rekordbox or mobile device. For the authenticated device, see "About an authenticated device for CloudDirectPlay" (page 8). For the mobile device, see "Using CloudDirectPlay" (page 13).
For CloudDirectPlay compatible DJ equipment, refer to the FAQ.
rekordbox.com/en/support/faq/clouddirect-7/#faq-q700150
The illustrations and operating procedures in this chapter are examples of the CDJ-3000X.

### Wireless LAN (Wi-Fi) connection

- Connect the multi players to the router via wireless LAN (Wi-Fi).
- To use PRO DJ LINK, connect the multi players and DJ mixer with LAN cables using a switching hub.

[Screenshot: Network connection diagram showing Internet at the top, connected to a Wireless LAN (Wi-Fi) router (Access Point), which connects wirelessly to two CDJ-3000X multi players and via a switching hub to a DJ mixer. The multi players and DJ mixer are also connected via the switching hub using LAN cables for PRO DJ LINK.]

## Cheking the Internet connection

When CloudDirectPlay is available, the Internet connection icon is displayed on the [SOURCE] screen as shown below.

[Screenshot: CDJ SOURCE screen showing "NO DEVICE" with an Internet connection icon (globe) in the upper-right corner. Player 1 indicator, 00:00 time display, "Not Loaded." status, and 0.00% / BPM display at the bottom.]

If the Internet connection icon is grayed out as (grayed out icon), CloudDirectPlay is not available. Check the Internet connection.

## Logging in to CloudDirectPlay

### Using an authenticated device

1. Insert the device authenticated for CloudDirectPlay to DJ equipment.
   When a valid authenticated device is inserted, it will be added as a source on the [SOURCE] screen.

2. Click [LOG IN] in the information area of the [SOURCE] screen.

[Screenshot: CDJ SOURCE screen showing two sources - a CLOUD source with "DJ Profile Name (USB)" selected/highlighted, and a USB source with "Device Name". On the right side: Songs, Playlists options, Status showing "NOT CONNECTED", a "LOG IN" button, and a "MY SETTINGS LOAD" button. Player 1, 00:00, "Not Loaded.", 0.00%, BPM at the bottom.]

CloudDirectPlay will be available.

### Not using an authenticated device

#### QR code login

1. Press the [SOURCE] button.
   The [SOURCE] screen will appear.

2. Select [Cloud Log In], and touch [LOG IN].

3. Use the built-in camera of your mobile device to scan the QR code.

4. Follow the instructions displayed on your mobile device to log in.
   rekordbox CloudDirectPlay will become available.
   - You can also log in by selecting the player to log in from [Cloud Log In] on the [SOURCE] screen.

[Screenshot: CDJ SOURCE screen showing "Cloud Log In" selected as a source. On the right side: "Please select a device to log in." with radio button options for AUTO SELECT, PLAYER No.1, PLAYER No.2, PLAYER No.3 (selected), PLAYER No.4, PLAYER No.5, PLAYER No.6, and a "LOG IN" button. Player 3 indicator at the bottom showing 03:25 time, waveform display, 4A key, 0.00%, 174.0 BPM.]

#### NFC login

1. When the NFC login indicator on the front panel of the DJ equipment is lit, hold your NFC-supported mobile device over the NFC login reader to scan the NFC tag.

[Screenshot: Front panel of CDJ-3000X DJ equipment with the NFC login reader location highlighted in the center-bottom area.]

**Note**

- If the NFC login indicator on the front panel of the DJ equipment is off, logging into the cloud is not available.
- If the NFC login indicator on the front panel of the DJ equipment is off or you need to select the player to log in, touch [Cloud Log In] on the [SOURCE] screen, select the player to log in, touch [LOG IN], and then scan the NFC tag of your NFC-supported mobile device.
- To log in using NFC, install the latest version of rekordbox for iOS/Android.
- If you use rekordbox for iOS, touch the notification on your mobile device after scanning the NFC tag to complete the login.

**Hint**

- For DJ equipment supporting QR code login/NFC login, refer to the FAQ on the rekordbox website
  (https://rekordbox.com/en/support/faq/log-in-to-dj-equipment/#faq-q100114).

### To log out of CloudDirectPlay

To cancel CloudDirectPlay, click [LOG OUT] in the information area of the [SOURCE] screen.

## Using music files of CloudDirectPlay

When you select an item of CloudDirectPlay on the [SOURCE] screen, you can use music files uploaded on the cloud from the browse screen.
To download a music file to DJ equipment, it takes time according to the Internet speed. The download progress is displayed at the bottom of the screen.

[Screenshot: CDJ player display showing PLAYER 1, TRACK 01, A.HOT CUE, CONTINUE, TEMPO -10, BPM, MASTER indicators. QUANTIZE 1, BEAT JUMP 16 on the left. "NOW LOADING..." status in the center. KEY indicator on the right.]

After downloading the file, the following will be displayed on the DJ equipment.
The screen shows an example when selecting PLAYLIST while using Cloud Export in Free Plan.

[Screenshot: CDJ BROWSE screen showing navigation categories on the left: CLOUD, ARTIST, ALBUM, TRACK, KEY, PLAYLIST (selected), HISTORY, MATCHING, DATE ADDED. Center panel shows PLAYLIST with "Trial playlist - Cloud Library Sync" and "Cloud Export" folder. Right panel shows TRACK list: Green plants, 6AM (Original Mix), Get Down (Original Mix), Workin' feat. Leela D (Original Mix) with a download indicator, Extra Trippy (Original Mix), Trees (Original Mix). Player 1, 00:00, "Not Loaded.", 0.00%, BPM at the bottom.]

For details on Cloud Export, refer to Cloud Library Sync Operation Guide.
rekordbox.com/en/download/#manual

**Note**

- The following functions can be used only with music files being loaded from CloudDirectPlay.
  CUE
  HOT CUE
- The following functions cannot be used because a music file is downloaded one by one from the TRACK list.
  TRACK SEARCH
  PLAYMODE(CONTINUE)
- Following functions cannot be used on CloudDirectPlay.
  TOUCH PREVIEW
  HOT CUE BANK
  INTELLIGENT PLAYLIST
- While using CloudDirectPlay, track information changed on other CloudDirectPlay or Cloud Library Sync with the same account cannot be reflected. To update with changes, log out from CloudDirecPlay, and then log in again.

# Others

## Troubleshooting

Before making inquiries about operations or technical issues, refer to troubleshooting below, or check the [FAQ] for each DJ equipment/rekordbox.

### Music files cannot be displayed or loaded.

On the supported DJ equipment, the music files may not be displayed on the screen or may not be loaded. There are possible causes as follows.

#### Cloud Sync is incomplete

If the Cloud Library Sync between rekordbox for Mac/Windows or rekordbox for iOS/Android and the library in the cloud has not finished, the music file will not be displayed on the browse screen. Wait for the sync to finish, then the music file should display.

#### The music file has not been uploaded to cloud storage

Only music files that you have uploaded to cloud storage will be displayed on the browse screen.
If the upload is not complete, the music file cannot be loaded.
Upload the music files you want to use to cloud storage beforehand, then use them once the upload is complete.
You can upload with rekordbox for Mac/Windows or rekordbox for iOS/Android.

#### Impact of cloud storage maintenance or failure

When cloud storage is undergoing maintenance or experiencing problems, music files cannot be loaded. Try again when the cloud storage is back online.
You can check the status of these issues on the cloud storage service websites below.
https://status.dropbox.com/
https://www.google.co.jp/appsstatus/dashboard/

### The USB storage device or SD memory card does not have enough space.

When using CloudDirectPlay, rekordbox temporarily stores audio files downloaded from Dropbox onto a USB storage device or SD memory card. So, the required free space depends on the number and size of the downloaded files.
The approximate size of an audio file for one track is shown below. It varies depending on the length of the track and the file format though.

- MP3 format: 12 MB (6-minute track at 320 kbps bit rate)
- WAV format: 66 MB (6-minute track with CD quality)

## Trademarks and licenses

- rekordbox is a trademark or registered trademark of AlphaTheta Corporation.
- Dropbox is a trademark or registered trademark of Dropbox, Inc.
- Windows is a registered trademark of Microsoft Corporation in the U.S. and other countries.
- Mac and macOS are trademarks of Apple Inc., registered in the U.S. and other countries and regions.
- iOS is trademarks or registered trademarks of Cisco in the U.S. and other countries and regions.
- "Google", the "Google Logo", and "Google Drive" are trademarks or registered trademarks of Google LLC.
- Android is a trademark or registered trademark of Google LLC.
- Wi-Fi is a registered trademark of Wi-Fi Alliance.

Other product, technology and company names, etc. mentioned herein are trademarks or registered trademarks of their respective owners.

(C) 2024 AlphaTheta Corporation.

## Related Documents

- [faq/usb-and-devices.md](../faq/usb-and-devices.md) (devices, pro-dj-link, usb)
- [guides/device-library-backup.md](device-library-backup.md) (devices, usb)
- [guides/pro-dj-link-setup.md](pro-dj-link-setup.md) (devices, pro-dj-link)
- [guides/usb-export.md](usb-export.md) (devices, usb)
- [manual/10-mobile-devices.md](../manual/10-mobile-devices.md) (cloud, devices)
- [manual/13-export-pro-dj-link.md](../manual/13-export-pro-dj-link.md) (pro-dj-link, usb)
- [manual/14-export-playing.md](../manual/14-export-playing.md) (playback, usb)
- [manual/15-export-lan.md](../manual/15-export-lan.md) (devices, pro-dj-link)
