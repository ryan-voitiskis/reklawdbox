#!/bin/bash
# Download official rekordbox PDF documentation (7.x plus related legacy guides).
# Can be run from any directory; the script changes to its own folder.

set -euo pipefail
cd "$(dirname "$0")"

echo "Downloading rekordbox 7 official documentation..."
echo ""

urls=(
  "https://cdn.rekordbox.com/files/20251202174516/rekordbox7.2.8_manual_EN.pdf"
  "https://cdn.rekordbox.com/files/20241213141709/rekordbox7.0.7_introduction_EN.pdf"
  "https://cdn.rekordbox.com/files/20251202174725/rekordbox7.2.8_cloud_library_sync_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20250908133738/rekordbox7.2.2_CloudDirectPlay_EN.pdf"
  "https://cdn.rekordbox.com/files/20241216130922/rekordbox7.0.7_lighting_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20260210150320/rekordbox-lighting-available-fixtures.pdf"
  "https://cdn.rekordbox.com/files/20241203210634/rekordbox7.0.5_Phrase_Edit_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20200918151433/rekordbox6.1.1_edit_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20241203185046/rekordbox7.0.5_video_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20241203185031/rekordbox7.0.5_dvs_setup_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20260127164936/rekordbox7.2.10_streaming_service_usage_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20241203210623/rekordbox7.0.5_midi_learn_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20251202174704/rekordbox7.2.8_pad_editor_operation_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20241203185020/rekordbox7.0.5_default_keyboard_shortcut_reference_EN.pdf"
  "https://cdn.rekordbox.com/files/20230711173001/rekordbox6.7.4_device_library_backup_guide_EN.pdf"
  "https://cdn.rekordbox.com/files/20251021171528/USB_export_guide_en_251007.pdf"
  "https://cdn.rekordbox.com/files/20251117092919/PRODJLINK_SetupGuide_ver2_en.pdf"
  "https://cdn.rekordbox.com/files/20200312171207/rekordbox5.3.0_connection_guide_for_performance_mode_EN.pdf"
  "https://cdn.rekordbox.com/files/20200410160904/xml_format_list.pdf"
)

for url in "${urls[@]}"; do
  filename=$(basename "$url")
  if [[ -f "$filename" ]]; then
    echo "SKIP  $filename (already exists)"
  else
    echo "GET   $filename"
    curl -sLO "$url"
  fi
done

echo ""
echo "Done. $(ls -1 *.pdf 2>/dev/null | wc -l | tr -d ' ') PDFs in $(pwd)"
