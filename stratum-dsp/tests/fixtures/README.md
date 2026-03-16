# Test Fixtures

This directory contains audio test files for integration testing.

## Required Fixtures

For comprehensive testing, the following fixtures should be added:

### 1. `120bpm_4bar.wav`

- **Description**: Simple kick drum pattern at 120 BPM
- **Duration**: 4 bars (8 seconds at 120 BPM)
- **Purpose**: Validate BPM detection and onset detection
- **Expected Results**:
  - BPM: 120.0 ± 2 BPM
  - Onsets: ~16 (4 beats per bar × 4 bars)

### 2. `128bpm_4bar.wav`

- **Description**: Another tempo for BPM detection validation
- **Duration**: 4 bars (7.5 seconds at 128 BPM)
- **Purpose**: Validate BPM detection across different tempos
- **Expected Results**:
  - BPM: 128.0 ± 2 BPM
  - Onsets: ~16

### 3. `cmajor_scale.wav`

- **Description**: Simple C major scale (C-D-E-F-G-A-B-C)
- **Duration**: ~10 seconds
- **Purpose**: Key detection validation (Phase 1D)
- **Expected Results**:
  - Key: C Major
  - Key confidence: > 0.8

### 4. `mixed_silence.wav`

- **Description**: Audio with leading/trailing silence and gaps
- **Duration**: ~15 seconds (5s silence, 5s audio, 5s silence)
- **Purpose**: Silence detection and trimming validation
- **Expected Results**:
  - Trimmed duration: ~5 seconds
  - Silence regions detected correctly

## Generating Test Fixtures

### Option 1: Use the Generation Script (Recommended)

A Python script is provided to generate all fixtures automatically:

```bash
# Install dependencies (one-time)
pip install -r scripts/requirements.txt
# OR
pip install numpy soundfile

# Generate all fixtures
python scripts/generate_fixtures.py
```

The script will generate all 4 fixtures in `tests/fixtures/`:

- `120bpm_4bar.wav` (8 seconds, ~16 onsets expected)
- `128bpm_4bar.wav` (7.5 seconds, ~16 onsets expected)
- `cmajor_scale.wav` (4 seconds, C Major key expected)
- `mixed_silence.wav` (15 seconds, 5s audio + 10s silence)

### Option 2: Manual Generation

If you prefer to generate fixtures manually, you can use the script as a reference:
See `scripts/generate_fixtures.py` for the implementation details.

### Option 2: Use Royalty-Free Samples

- **Splice**: https://splice.com (requires account)
- **Pexels Audio**: https://www.pexels.com/audio/ (royalty-free)
- **Freesound**: https://freesound.org (check license)

### Option 3: MusicBrainz Metadata

Use tracks with known BPM/key from MusicBrainz:

- Search for tracks with verified metadata
- Download test clips (ensure licensing allows use)

## Current Status

✅ **Fixtures generated** - Run `python scripts/generate_fixtures.py` to regenerate if needed.

## Adding Fixtures

1. Generate or download test audio files
2. Place in `tests/fixtures/` directory
3. Update `.gitignore` if files are large (consider Git LFS)
4. Document expected results in this README
5. Add integration tests in `tests/integration_tests.rs`

## File Size Considerations

- Keep individual files < 1MB when possible
- For larger files, consider using Git LFS
- Or generate fixtures on-the-fly in tests (slower but no storage)

## License

Test fixtures should be:

- Public domain, or
- Licensed for testing use, or
- Generated synthetically (no license concerns)
