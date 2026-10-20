import { test } from 'node:test';
import assert from 'node:assert/strict';
import { moveInList } from './list-keys.ts';
import { rovingTarget } from './roving-keys.ts';

// Just enough of an element for the two helpers: matching by a fixed set of
// selectors, a parent, children, focus and click.
if (!globalThis.Element) globalThis.Element = function Element() {};
function node(selectors, children = [], extra = {}) {
  const element = Object.assign(Object.create(globalThis.Element.prototype), { selectors, children, parent: null, focused: false, clicked: false, ...extra });
  for (const child of children) child.parent = element;
  element.matches = selector => selector.split(',').some(part => selectors.includes(part.trim()));
  element.closest = selector => { for (let at = element; at; at = at.parent) if (at.matches(selector)) return at; return null; };
  element.contains = other => { for (let at = other; at; at = at.parent) if (at === element) return true; return false; };
  element.querySelectorAll = selector => { const found = []; const walk = at => { for (const child of at.children) { if (child.matches(selector)) found.push(child); walk(child); } }; walk(element); return found; };
  element.focus = () => { element.focused = true; };
  element.click = () => { element.clicked = true; };
  return element;
}
const press = (key, target, mods = {}) => ({ key, target, currentTarget: target, altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, ...mods });

test('a list moves its focus with the arrows and stops at its ends', () => {
  const rows = [node(['.row']), node(['.row']), node(['.row'])];
  const list = node(['.list'], rows);
  assert.equal(moveInList(press('ArrowDown', rows[0]), list, '.row'), true);
  assert.equal(rows[1].focused, true);
  assert.equal(moveInList(press('ArrowDown', rows[2]), list, '.row'), true);
  assert.equal(rows[2].focused, true, 'the last row keeps it');
  assert.equal(moveInList(press('Home', rows[2]), list, '.row'), true);
  assert.equal(rows[0].focused, true);
  assert.equal(moveInList(press('ArrowDown', rows[0], { shiftKey: true }), list, '.row'), false, 'Shift+arrow selects a range instead');
  assert.equal(moveInList(press('a', rows[0]), list, '.row'), false);
});

test('tabs and option groups choose as the arrows move, and wrap', () => {
  const tabs = [node(['[role="tab"]']), node(['[role="tab"]']), node(['[role="tab"]'])];
  node(['[role="tablist"]'], tabs);
  assert.equal(rovingTarget(press('ArrowRight', tabs[0])), tabs[1]);
  assert.equal(rovingTarget(press('ArrowLeft', tabs[0])), tabs[2], 'wraps');
  assert.equal(rovingTarget(press('End', tabs[0])), tabs[2]);
  assert.equal(rovingTarget(press('ArrowDown', tabs[0])), undefined, 'a strip runs sideways');
  const sections = [node(['button']), node(['button'])];
  node(['[data-arrow-nav]'], sections);
  assert.equal(rovingTarget(press('ArrowDown', sections[0])), sections[1], 'a navigation list runs down');
  assert.equal(rovingTarget({ ...press('ArrowRight', tabs[0]), defaultPrevented: true }), undefined, 'a widget with its own arrows is left alone');
});
