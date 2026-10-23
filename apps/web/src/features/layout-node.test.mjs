import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dockPosition, keyedRatio, keyedTab, splitGrid, tabsAround } from './layout-node.ts';
import { paneDisplayTitle } from './pane-context.ts';
import { SPLIT_GAP } from '../layout/layout-engine.ts';

test('a dragged pane docks beside the nearest edge within its margin, else as a tab', () => {
  // 600 x 300 with no header: margins are a third, 200 across and 100 down.
  assert.equal(dockPosition(10, 150, 600, 300), 'left');
  assert.equal(dockPosition(190, 150, 600, 300), 'left', 'a third of the way in still splits');
  assert.equal(dockPosition(590, 150, 600, 300), 'right');
  assert.equal(dockPosition(300, 10, 600, 300), 'top');
  assert.equal(dockPosition(300, 295, 600, 300), 'bottom');
  assert.equal(dockPosition(300, 150, 600, 300), 'center');
  assert.equal(dockPosition(200, 150, 600, 300), 'center', 'the margin edge itself is the middle');
  // In a corner the edge nearer for its side's margin wins.
  assert.equal(dockPosition(20, 50, 600, 300), 'left');
  assert.equal(dockPosition(150, 5, 600, 300), 'top');
});

test('over the tab strip a dragged pane joins the tabs, and the edges are measured below it', () => {
  // 600 x 338 with a 38 header: the content is 600 x 300.
  assert.equal(dockPosition(5, 20, 600, 338, 38), 'center', 'even at the strip\'s edge');
  assert.equal(dockPosition(300, 40, 600, 338, 38), 'top');
  assert.equal(dockPosition(300, 137, 600, 338, 38), 'top');
  assert.equal(dockPosition(300, 188, 600, 338, 38), 'center');
  assert.equal(dockPosition(300, 330, 600, 338, 38), 'bottom');
});

test('a split lays out its sides around the divider, along its direction', () => {
  assert.deepEqual(splitGrid('horizontal', 0.25), { gridTemplateColumns: `minmax(0, 0.25fr) ${SPLIT_GAP}px minmax(0, 0.75fr)`, gridTemplateRows: 'minmax(0, 1fr)' });
  assert.deepEqual(splitGrid('vertical', 0.5), { gridTemplateRows: `minmax(0, 0.5fr) ${SPLIT_GAP}px minmax(0, 0.5fr)`, gridTemplateColumns: 'minmax(0, 1fr)' });
});

test('the arrows along a split nudge its divider, Shift further, and Home centres it', () => {
  assert.equal(keyedRatio('ArrowRight', 'horizontal', 0.5, false), 0.525);
  assert.equal(keyedRatio('ArrowLeft', 'horizontal', 0.5, true), 0.4);
  assert.equal(keyedRatio('ArrowDown', 'vertical', 0.5, false), 0.525);
  assert.equal(keyedRatio('ArrowUp', 'horizontal', 0.5, false), undefined, 'not along this split');
  assert.equal(keyedRatio('Home', 'vertical', 0.8, false), 0.5);
  assert.equal(keyedRatio('Enter', 'horizontal', 0.5, false), undefined);
});

test('tab keys wrap at the ends and jump to them', () => {
  assert.equal(keyedTab('ArrowRight', 2, 3), 0);
  assert.equal(keyedTab('ArrowLeft', 0, 3), 2);
  assert.equal(keyedTab('Home', 2, 3), 0);
  assert.equal(keyedTab('End', 0, 3), 2);
  assert.equal(keyedTab('ArrowRight', 0, 0), undefined, 'no tabs');
  assert.equal(keyedTab('Tab', 0, 3), undefined);
});

test('the tabs beside one are split for closing in bulk', () => {
  const tab = id => ({ type: 'pane', id, kind: 'editor' });
  const tabs = [tab('a'), tab('b'), tab('c'), tab('d')];
  const around = tabsAround(tabs, 'b');
  assert.deepEqual(around.left.map(t => t.id), ['a']);
  assert.deepEqual(around.right.map(t => t.id), ['c', 'd']);
  assert.deepEqual(around.others.map(t => t.id), ['a', 'c', 'd']);
  assert.deepEqual(tabsAround(tabs, 'gone'), { left: [], right: [], others: tabs });
});

test('a pane is shown under its own title, or a translated built-in name', () => {
  const t = text => `<${text}>`;
  assert.equal(paneDisplayTitle({ type: 'pane', id: 's', kind: 'agent_chat', title: 'Fix login', metadata: { session_id: 'x' } }, t), 'Fix login', 'a session title is its own');
  assert.equal(paneDisplayTitle({ type: 'pane', id: 'f', kind: 'editor', title: 'Changes', metadata: { path: 'Changes' } }, t), 'Changes', 'so is a file name, even one that matches the UI');
  assert.equal(paneDisplayTitle({ type: 'pane', id: 'g', kind: 'git_diff', title: 'Changes' }, t), '<Changes>');
  assert.equal(paneDisplayTitle({ type: 'pane', id: 'g', kind: 'git_diff' }, t), '<Git Changes>');
  assert.equal(paneDisplayTitle({ type: 'pane', id: 'a', kind: 'agent_chat' }, t), '<Agent session>');
});
