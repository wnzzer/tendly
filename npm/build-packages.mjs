// Turns the release tarballs into publishable npm packages: one per platform
// holding a binary, plus the package users actually name, which holds only the
// shim and depends on all of them optionally.
//
//   node npm/build-packages.mjs <version> <dir-of-tarballs> [out]
//
// Run locally against artifacts downloaded from a workflow run to inspect
// exactly what a publish would push.
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCOPE = '@wnzzer';
const NAME = 'tendly';
const BINARY = 'tendly-server';
const REPOSITORY = 'https://github.com/wnzzer/tendly';
// One source in docs/, published under the name a registry reader expects.
const NOTICES_SOURCE = 'docs/third-party-notices.md';
const NOTICES = 'THIRD-PARTY-NOTICES.md';

// A rust target names a triple; npm matches `${process.platform}-${process.arch}`.
// Publishing under the npm spelling is what lets the shim resolve a package
// name straight from `process`, with no second table to keep in step.
const TARGETS = [
  { triple: 'aarch64-apple-darwin', key: 'darwin-arm64', os: 'darwin', cpu: 'arm64' },
  { triple: 'x86_64-apple-darwin', key: 'darwin-x64', os: 'darwin', cpu: 'x64' },
  { triple: 'x86_64-unknown-linux-musl', key: 'linux-x64', os: 'linux', cpu: 'x64' },
  { triple: 'aarch64-unknown-linux-musl', key: 'linux-arm64', os: 'linux', cpu: 'arm64' },
  { triple: 'x86_64-pc-windows-msvc', key: 'win32-x64', os: 'win32', cpu: 'x64' },
];
const binaryName = (target) => (target.os === 'win32' ? `${BINARY}.exe` : BINARY);

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const [version, tarballDir, outArg] = process.argv.slice(2);
if (!version || !tarballDir) {
  console.error('usage: node npm/build-packages.mjs <version> <dir-of-tarballs> [out]');
  process.exit(2);
}
const out = path.resolve(outArg ?? path.join(root, 'dist-npm'));
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

const shared = {
  version,
  description: 'Remote multi-agent development workspace for the official Claude Code and Codex CLIs',
  license: 'MIT',
  homepage: REPOSITORY,
  repository: { type: 'git', url: `git+${REPOSITORY}.git` },
  engines: { node: '>=24' },
};

const extractRoot = mkdtempSync(path.join(tmpdir(), 'tendly-npm-'));
try {
  for (const target of TARGETS) {
    const tarball = `${NAME}-${version}-${target.triple}.tar.gz`;
    const staged = path.join(extractRoot, target.triple);
    mkdirSync(staged, { recursive: true });
    // Relative to the tarball directory: GNU tar reads the `C:` of a Windows
    // path as a remote host, and a relative path has no drive to misread.
    const from = path.resolve(tarballDir);
    execFileSync('tar', ['xzf', tarball, '-C', path.relative(from, staged).split(path.sep).join('/')], { cwd: from, stdio: 'inherit' });

    // The tarball holds one directory named for the build; find it rather than
    // rebuilding the name, so a rename upstream fails loudly here.
    const [unpacked] = readdirSync(staged);
    const binary = path.join(staged, unpacked, binaryName(target));

    const dir = path.join(out, `${NAME}-${target.key}`);
    mkdirSync(path.join(dir, 'bin'), { recursive: true });
    cpSync(binary, path.join(dir, 'bin', binaryName(target)));
    // npm records the mode in the tarball, so setting it here is what makes the
    // installed file runnable.
    chmodSync(path.join(dir, 'bin', binaryName(target)), 0o755);
    cpSync(path.join(root, 'LICENSE'), path.join(dir, 'LICENSE'));
    // The binary embeds the web bundle, which embeds MIT-licensed brand
    // vectors. Their licence requires the notice to travel with any copy, and
    // an npm package is a copy.
    cpSync(path.join(root, NOTICES_SOURCE), path.join(dir, NOTICES));
    writeFileSync(
      path.join(dir, 'package.json'),
      `${JSON.stringify(
        {
          name: `${SCOPE}/${NAME}-${target.key}`,
          ...shared,
          // npm reads these before downloading, so a machine only ever fetches
          // the binary it can run.
          os: [target.os],
          cpu: [target.cpu],
          // No `exports`: the shim resolves ./package.json, which an exports
          // map would otherwise have to re-declare.
          files: ['bin/', 'LICENSE', NOTICES],
        },
        null,
        2,
      )}\n`,
    );
  }
} finally {
  rmSync(extractRoot, { recursive: true, force: true });
}

function mainPackage(name, bin) {
  const dir = path.join(out, name);
  mkdirSync(path.join(dir, 'bin'), { recursive: true });
  cpSync(path.join(here, 'shim.cjs'), path.join(dir, 'bin', `${NAME}.js`));
  cpSync(path.join(root, 'LICENSE'), path.join(dir, 'LICENSE'));
  cpSync(path.join(root, 'README.md'), path.join(dir, 'README.md'));
  cpSync(path.join(root, NOTICES_SOURCE), path.join(dir, NOTICES));
  writeFileSync(
    path.join(dir, 'package.json'),
    `${JSON.stringify(
      {
        name: `${SCOPE}/${name}`,
        ...shared,
        keywords: ['claude-code', 'codex', 'agent', 'workspace', 'cli'],
        bin: Object.fromEntries(bin.map((command) => [command, `bin/${NAME}.js`])),
        // Pinned exactly: these ship a binary built from this very commit, so a
        // range could pair a shim with an executable it was never tested against.
        optionalDependencies: Object.fromEntries(
          TARGETS.map((target) => [`${SCOPE}/${NAME}-${target.key}`, version]),
        ),
        files: ['bin/', 'LICENSE', NOTICES, 'README.md'],
      },
      null,
      2,
    )}\n`,
  );
}
mainPackage(NAME, [NAME]);

// Platform packages first: the main package names them as dependencies, and a
// registry that has not seen them yet resolves an install to nothing.
const order = [...TARGETS.map((target) => `${NAME}-${target.key}`), NAME];
writeFileSync(path.join(out, 'publish-order.txt'), `${order.join('\n')}\n`);
console.log(`built ${order.length} packages for ${version} in ${out}`);
for (const name of order) console.log(`  ${SCOPE}/${name}`);
