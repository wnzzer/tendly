import test from 'node:test';
import assert from 'node:assert/strict';
import { TEXT_SCALES, pinchStep, stepTextScale, textScale } from './appearance.ts';

test('a step moves through the text sizes and stop at either end', () => {
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

test('only a deliberate pinch steps the text size', () => {
  assert.equal(pinchStep(100, 125), 0, 'fingers drifting apart are not a pinch');
  assert.equal(pinchStep(100, 140), 1);
  assert.equal(pinchStep(100, 80), 0);
  assert.equal(pinchStep(100, 70), -1);
  assert.equal(pinchStep(0, 300), 0, 'a pinch that already stepped waits for the fingers to lift');
});
