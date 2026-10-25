import test from 'node:test';
import assert from 'node:assert/strict';
import { TEXT_SCALES, stepTextScale, textScale } from './appearance.ts';

test('A− and A+ step through the text sizes and stop at either end', () => {
  textScale.value = 'default';
  stepTextScale(1);
  assert.equal(textScale.value, 'large');
  stepTextScale(-1); stepTextScale(-1);
  assert.equal(textScale.value, 'small');
  stepTextScale(-1);
  assert.equal(textScale.value, TEXT_SCALES[0], 'nothing below the smallest');
  for (const _ of TEXT_SCALES) stepTextScale(1);
  assert.equal(textScale.value, TEXT_SCALES.at(-1), 'nothing above the largest');
});
