import assert from 'node:assert/strict'
import test from 'node:test'

import { reviewGuide, reviewSheet } from './genre_truth_review_material.mjs'

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
  const guide = reviewGuide([row])
  for (const output of [sheet, guide]) {
    assert.match(output, /GI01-01/)
    assert.match(output, /Artist/)
    assert.match(output, /Title/)
    assert.doesNotMatch(output, /SECRET_SAMPLING_LABEL/)
    assert.doesNotMatch(output, /SECRET_CURRENT_GENRE/)
    assert.doesNotMatch(output, /SECRET_MODEL_PREDICTION/)
  }
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
