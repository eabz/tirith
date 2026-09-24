#!/usr/bin/env node
// Read-only scanner for unused assets, docs and scripts. Never edits files.
// Usage: node scan.mjs <repo-root> [--json] [--min-kb=0]
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, extname, join, relative, sep } from 'node:path';

const args = process.argv.slice(2);
const root = args.find((a) => !a.startsWith('--')) ?? '.';
const asJson = args.includes('--json');
const minKb = Number(args.find((a) => a.startsWith('--min-kb='))?.split('=')[1] ?? 0);

const SKIP_DIRS = new Set(['node_modules', '.next', '.git', 'graphify-out', '.agents', '.claude', '.serena', '.vercel', 'coverage', 'dist', 'build', 'out']);
const ASSET_EXT = new Set(['.png', '.jpg', '.jpeg', '.webp', '.avif', '.gif', '.svg', '.ico', '.mp4', '.webm', '.mov', '.mp3', '.pdf', '.woff', '.woff2', '.ttf', '.otf', '.vtt', '.json', '.txt', '.xml', '.csv', '.docx', '.zip']);
const TEXT_EXT = new Set(['.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs', '.css', '.scss', '.json', '.jsonc', '.md', '.mdx', '.html', '.yml', '.yaml', '.toml', '.py', '.sh', '.txt', '.xml', '.tex', '.sql']);
// Files that frameworks or crawlers load by convention, not by reference.
const CONVENTION = /(^|\/)(favicon|icon|apple-icon|apple-touch-icon|robots|sitemap|manifest|site\.webmanifest|browserconfig|opengraph-image|twitter-image|humans|security|ads|llms)[^/]*$|(^|\/)\.well-known\//i;

function walk(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === '.DS_Store') continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) walk(full, out);
    } else out.push(full);
  }
  return out;
}

const rel = (f) => relative(root, f).split(sep).join('/');
const files = walk(root);
const kb = (f) => Math.round(statSync(f).size / 1024);
const read = (f) => {
  try {
    return statSync(f).size < 2_000_000 ? readFileSync(f, 'utf8') : '';
  } catch {
    return '';
  }
};

// Reference corpora: code/config (real usage) and docs (mention only).
let code = '';
let docs = '';
for (const f of files) {
  const r = rel(f);
  if (r.startsWith('public/') || !TEXT_EXT.has(extname(f)) || r === 'bun.lock') continue;
  if (r.startsWith('docs/') || extname(f) === '.md') docs += `\n${read(f)}`;
  else code += `\n${read(f)}`;
}

// Prefixes of dynamic paths like `/media/${slug}.webp` or "/fotos/" + id.
const dynamicPrefixes = new Set();
for (const m of code.matchAll(/[`'"](\/?[\w\-./]*\/)[\w\-.]*(?:\$\{|['"]\s*\+)/g)) {
  if (m[1].length > 1) dynamicPrefixes.add(m[1].replace(/^\//, ''));
}

const findings = { unused: [], dynamic: [], docsOnly: [], convention: [] };

function classify(file, publicPath) {
  const name = basename(file);
  // Match the served path or the file name as a whole token (after a quote or slash),
  // so short names like `home.webp` are not "used" by unrelated `home.stats` code.
  const token = new RegExp(`['"\`/(]${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`);
  const hit = (corpus) => corpus.includes(publicPath) || token.test(corpus);
  if (hit(code)) return null;
  const item = { path: rel(file), kb: kb(file) };
  if (CONVENTION.test(publicPath)) return ['convention', item];
  const relToPublic = publicPath.replace(/^\//, '');
  if ([...dynamicPrefixes].some((p) => relToPublic.startsWith(p))) return ['dynamic', item];
  if (hit(docs)) return ['docsOnly', item];
  return ['unused', item];
}

for (const f of files) {
  const r = rel(f);
  const isPublic = r.startsWith('public/');
  const isSrcAsset = !isPublic && /^(src|app|content|assets)\//.test(r) && ASSET_EXT.has(extname(f)) && !['.json', '.txt', '.csv'].includes(extname(f));
  if (!isPublic && !isSrcAsset) continue;
  const result = classify(f, isPublic ? r.slice('public'.length) : r);
  if (result && result[1].kb >= minKb) findings[result[0]].push(result[1]);
}

// Scripts that nothing runs: not in package.json, CI, docs or other code.
const pkg = read(join(root, 'package.json'));
findings.orphanScripts = files
  .map(rel)
  .filter((r) => /^(scripts|tools)\/.+\.(ts|js|mjs|cjs|py|sh)$/.test(r))
  .filter((r) => {
    const name = basename(r);
    const stem = name.replace(/\.[^.]+$/, '');
    const others = code.replace(read(join(root, r)), '');
    return !(pkg.includes(name) || pkg.includes(stem) || others.includes(name) || others.includes(`/${stem}'`) || others.includes(`/${stem}"`) || docs.includes(name));
  })
  .map((path) => ({ path, kb: kb(join(root, path)) }));

// Docs not linked from any other markdown, AGENTS.md or README.
const allMd = files.map(rel).filter((r) => r.endsWith('.md'));
const mdText = allMd.map((r) => [r, read(join(root, r))]);
findings.orphanDocs = allMd
  .filter((r) => r.startsWith('docs/') && !/(^|\/)README\.md$/i.test(r))
  .filter((r) => !mdText.some(([other, text]) => other !== r && text.includes(basename(r))))
  .map((path) => ({ path, kb: kb(join(root, path)) }));

// Last commit date helps separate stale leftovers from fresh work.
const lastCommit = (p) => {
  try {
    return execFileSync('git', ['-C', root, 'log', '-1', '--format=%cs', '--', p], { encoding: 'utf8' }).trim() || 'sin commit';
  } catch {
    return '?';
  }
};
for (const key of ['unused', 'docsOnly', 'orphanScripts', 'orphanDocs']) {
  for (const item of findings[key]) item.lastCommit = lastCommit(item.path);
  findings[key].sort((a, b) => b.kb - a.kb);
}

if (asJson) {
  console.log(JSON.stringify(findings, null, 2));
} else {
  const labels = {
    unused: 'Sin referencia (candidatos a borrar)',
    dynamic: 'Posible ruta dinámica (revisar a mano)',
    docsOnly: 'Solo mencionados en docs',
    convention: 'Cargados por convención (se conservan)',
    orphanScripts: 'Scripts que nada ejecuta',
    orphanDocs: 'Docs que nadie enlaza',
  };
  for (const [key, label] of Object.entries(labels)) {
    const list = findings[key];
    const total = list.reduce((s, i) => s + i.kb, 0);
    console.log(`\n## ${label}: ${list.length} (${(total / 1024).toFixed(1)} MB)`);
    for (const i of list) console.log(`  ${String(i.kb).padStart(7)} KB  ${i.path}${i.lastCommit ? `  (${i.lastCommit})` : ''}`);
  }
  console.log(`\nPrefijos dinámicos detectados: ${[...dynamicPrefixes].slice(0, 20).join(', ') || 'ninguno'}`);
}
