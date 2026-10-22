import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shellHeight, shellTop } from './viewport-height.ts';

test('the page is only resized for the one thing dvh cannot see', () => {
  // A keyboard: half the screen, and the page is never told by anything else.
  assert.equal(shellHeight({ height: 420, scale: 1 }, 896), 420);
  // Nothing in the way, so nothing to do: dvh is left to do its job.
  assert.equal(shellHeight({ height: 896, scale: 1 }, 896), undefined);
  // A URL bar collapsing or appearing moves this by tens of pixels, which dvh
  // already handles. Reacting to it as well would resize the page twice.
  assert.equal(shellHeight({ height: 830, scale: 1 }, 896), undefined);
  // Pinching hides nothing; resizing the page to a pinch fights the gesture.
  assert.equal(shellHeight({ height: 300, scale: 2.5 }, 896), undefined);
  // A browser too old to say stays on dvh rather than guessing.
  assert.equal(shellHeight(undefined, 896), undefined);
  // Fractional heights are real on a phone, and a fractional pixel is not.
  assert.equal(shellHeight({ height: 419.6, scale: 1 }, 896), 420);
});

test('the shell follows the visible part of the page down while a keyboard is up', () => {
  // iOS pans to the caret instead of resizing: the shell moves with it, or it
  // ends above the keyboard with empty page beneath.
  assert.equal(shellTop({ height: 420, scale: 1, offsetTop: 310.4 }, 420), 310);
  // No keyboard, no fitting, and nothing to follow.
  assert.equal(shellTop({ height: 830, scale: 1, offsetTop: 40 }, undefined), 0);
  // A browser that resizes the page instead never pans it.
  assert.equal(shellTop({ height: 420, scale: 1, offsetTop: 0 }, 420), 0);
  assert.equal(shellTop({ height: 420, scale: 1 }, 420), 0);
});
