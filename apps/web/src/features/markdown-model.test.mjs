import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileReference, isLocalImageReference, linkProse, markdownBlocksWithLines } from './markdown-model.ts';

test('each block remembers the line it starts on, counted from zero', () => {
  const text = '# Title\n\nFirst line\nsecond line\n\n```ts\nconst a = 1;\n```\n- one\n- two';
  assert.deepEqual(markdownBlocksWithLines(text).map(({ block, line }) => [block.type, line]), [
    ['heading', 0], ['paragraph', 2], ['code', 5], ['list', 8],
  ]);
  // Windows line endings count the same lines.
  assert.deepEqual(markdownBlocksWithLines('a\r\n\r\nb').map(entry => entry.line), [0, 2]);
});

test('a dashed line under text makes a heading; on its own it is a rule', () => {
  assert.deepEqual(markdownBlocksWithLines('Title\n---').map(entry => entry.block), [{ type: 'heading', level: 2, text: 'Title' }]);
  assert.deepEqual(markdownBlocksWithLines('Title\n===').map(entry => entry.block), [{ type: 'heading', level: 1, text: 'Title' }]);
  assert.deepEqual(markdownBlocksWithLines('text\n\n***').map(entry => entry.block.type), ['paragraph', 'rule']);
});

test('a table needs a divider as wide as its header, and short rows are padded', () => {
  const [table] = markdownBlocksWithLines('| a | b | c |\n|:--|:--:|--:|\n| 1 | 2 |\n| x | y | z |\nafter').map(entry => entry.block);
  assert.deepEqual(table, {
    type: 'table', header: ['a', 'b', 'c'], align: ['left', 'center', 'right'],
    rows: [['1', '2', ''], ['x', 'y', 'z']],
  });
  // A divider of the wrong width is no table, just text.
  assert.equal(markdownBlocksWithLines('| a | b |\n|---|\n| 1 | 2 |')[0].block.type, 'paragraph');
  // One dash a cell is a divider, as GFM has it.
  assert.deepEqual(markdownBlocksWithLines('| 模块 | 估时 |\n|-|-|\n| 标签 | 3 天 |')[0].block, { type: 'table', header: ['模块', '估时'], align: [undefined, undefined], rows: [['标签', '3 天']] });
  assert.deepEqual(markdownBlocksWithLines('| a | b |\n|:-:|-:|')[0].block.align, ['center', 'right']);
});

test('only paths to image files inside the workspace count as local images', () => {
  for (const src of ['shot.png', './img/a.JPG?raw', 'docs/diagram.svg#top', '/abs/photo.webp']) assert.equal(isLocalImageReference(src), true, src);
  for (const src of ['https://x.test/a.png', '//cdn/a.png', 'data:image/png;base64,AA', 'notes.txt', '', '   ']) assert.equal(isLocalImageReference(src), false, src);
});

test('file paths in prose become references, with a line where one is given', () => {
  assert.deepEqual(fileReference('src/main.rs:42'), { path: 'src/main.rs', line: 42 });
  assert.deepEqual(fileReference('src/main.rs#L7-L9'), { path: 'src/main.rs', line: 7 });
  assert.deepEqual(fileReference('C:\\work\\a.ts'), { path: 'C:/work/a.ts' });
  assert.equal(fileReference('version 1.2'), undefined, 'not a path');
  assert.equal(fileReference('a//b.ts'), undefined);
});

test('prose keeps its text and links paths and web addresses inside it', () => {
  assert.deepEqual(linkProse('See src/app.ts:3 and https://example.com/docs.'), [
    { type: 'text', text: 'See ' },
    { type: 'file', text: 'src/app.ts:3', path: 'src/app.ts', line: 3, code: false },
    { type: 'text', text: ' and ' },
    { type: 'link', text: 'https://example.com/docs', href: 'https://example.com/docs' },
    { type: 'text', text: '.' },
  ]);
  assert.deepEqual(linkProse('nothing to link'), [{ type: 'text', text: 'nothing to link' }]);
  assert.deepEqual(linkProse(''), [{ type: 'text', text: '' }]);
});
