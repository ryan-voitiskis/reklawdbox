import assert from 'node:assert/strict'
import test from 'node:test'

import {
  blindReviewXml,
  reviewGuide,
  reviewSheet,
  validateReviewRoster,
} from './genre_truth_review_material.mjs'

const row = {
  position: 1,
  code: 'GI01-01',
  artist: 'Artist',
  title: 'Title',
  sampling_stratum_private: 'SECRET_SAMPLING_LABEL',
  current_genre_sampling_only: 'SECRET_CURRENT_GENRE',
  model_prediction_private: 'SECRET_MODEL_PREDICTION',
}

test('review material contains only whitelisted identity and answer fields', () => {
  const sheet = reviewSheet([row])
  const guide = reviewGuide([row], 'B02')
  for (const output of [sheet, guide]) {
    assert.match(output, /GI01-01/)
    assert.match(output, /Artist/)
    assert.match(output, /Title/)
    assert.doesNotMatch(output, /SECRET_SAMPLING_LABEL/)
    assert.doesNotMatch(output, /SECRET_CURRENT_GENRE/)
    assert.doesNotMatch(output, /SECRET_MODEL_PREDICTION/)
  }
  assert.match(guide, /Blind Review B02/)
})

test('review sheet escapes tab and newline identity content', () => {
  const sheet = reviewSheet([{
    ...row,
    artist: 'Artist\tName',
    title: 'A\nTitle',
  }])
  const lines = sheet.trimEnd().split('\n')
  assert.equal(lines.length, 2)
  assert.match(lines[1], /Artist Name/)
  assert.match(lines[1], /A Title/)
})

test('roster validation enforces the declared artist cap and unique releases', () => {
  const selected = [
    {
      ...row,
      track_id: '1',
      file_path: '/music/1.flac',
      artist_group: 'artist',
      release_group: 'release-1',
    },
    {
      ...row,
      track_id: '2',
      file_path: '/music/2.flac',
      artist_group: 'artist',
      release_group: 'release-2',
    },
  ]
  assert.doesNotThrow(() => validateReviewRoster(selected, 2))
  assert.throws(() => validateReviewRoster(selected, 1), /identity-diversity/)
  assert.throws(
    () =>
      validateReviewRoster([
        selected[0],
        { ...selected[1], release_group: 'release-1' },
      ], 2),
    /identity-diversity/,
  )
})

test('blind-review XML retains only identity and audio location', () => {
  const xml = [
    '<DJ_PLAYLISTS>',
    '  <COLLECTION Entries="1">',
    '    <TRACK TrackID="1" Name="Track" Artist="Artist" Album="Album" Genre="Techno" Comments="hidden" Rating="204" Location="file://localhost/Music/track.wav"/>',
    '  </COLLECTION>',
    '  <PLAYLISTS>',
    '    <NODE Type="1" Name="opaque">',
    '      <TRACK Key="1"/>',
    '    </NODE>',
    '  </PLAYLISTS>',
    '</DJ_PLAYLISTS>',
    '',
  ].join('\n')
  const scrubbed = blindReviewXml(xml)
  assert.match(
    scrubbed,
    /<TRACK TrackID="1" Name="Track" Artist="Artist" Album="Album" Location="file:\/\/localhost\/Music\/track\.wav"\/>/,
  )
  assert.match(scrubbed, /<TRACK Key="1"\/>/)
  assert.doesNotMatch(scrubbed, /Genre=|Comments=|Rating=/)
})
