// Feature-Sliced Design import-direction check (SPEC §13.1).
// A slice file may import only from its own layer or lower layers; the
// dependency direction is app -> pages -> widgets -> features -> entities ->
// shared. Run: node scripts/check-fsd.mjs [--self-test]
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = resolve(fileURLToPath(new URL('..', import.meta.url)), 'src');
const LAYERS = ['app', 'pages', 'widgets', 'features', 'entities', 'shared'];
const RANK = new Map(LAYERS.map((layer, index) => [layer, index]));

function layerOf(rel) {
  const first = rel.split(sep)[0];
  return LAYERS.includes(first) ? first : null;
}

// Resolve an import target to the FSD layer it reaches, or null when it does
// not cross into another layer (same-slice relative imports, bare packages).
function resolveLayer(fromFile, target, src) {
  if (target.startsWith('@/')) {
    const first = target.slice(2).split('/')[0];
    return LAYERS.includes(first) ? first : null;
  }
  if (target.startsWith('.')) {
    const dir = fromFile.slice(0, fromFile.lastIndexOf(sep));
    const rel = relative(src, resolve(join(dir, target)));
    return rel.startsWith('..') ? null : layerOf(rel);
  }
  return null;
}

function checkFile(file, src, violations) {
  const rel = relative(src, file);
  const layer = layerOf(rel);
  if (layer === null) return; // src-root files (main.tsx, test-setup.ts)
  const content = readFileSync(file, 'utf8');
  for (const match of content.matchAll(/from\s+['"]([^'"]+)['"]/g)) {
    const target = match[1];
    const targetLayer = resolveLayer(file, target, src);
    if (targetLayer !== null && RANK.get(targetLayer) < RANK.get(layer)) {
      violations.push({ file: rel, from: layer, to: targetLayer, target });
    }
  }
}

function scan(src) {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.(ts|tsx)$/.test(entry)) files.push(path);
    }
  };
  walk(src);
  const violations = [];
  for (const file of files) checkFile(file, src, violations);
  return { files: files.length, violations };
}

function selfTest() {
  // A throwaway src tree proving both directions fail/pass: alias imports,
  // relative upward imports, and the shared-layer ceiling.
  const root = mkdtempSync(join(process.env.TMPDIR ?? '/tmp', 'fsd-check-'));
  const write = (rel, content) => {
    const path = join(root, rel);
    mkdirSync(path.slice(0, path.lastIndexOf(sep)), { recursive: true });
    writeFileSync(path, content);
  };
  write('src/features/a.ts', `import { x } from '@/widgets/b';`);
  write('src/features/c.ts', `import { y } from '@/entities/d';`);
  write('src/widgets/b.ts', `import { z } from '../features/e';`);
  write('src/features/e.ts', 'export const z = 1;');
  write('src/entities/k.ts', `import { q } from '../features/e';`);
  write('src/entities/d.ts', "import { g } from './g'; export const y = g;");
  write('src/entities/g.ts', 'export const g = 1;');
  write('src/shared/h.ts', `import { i } from '@/pages/i';`);
  write('src/pages/i.ts', 'export const i = 1;');
  const { violations } = scan(join(root, 'src'));
  const ids = violations.map((v) => v.target).sort();
  // features->widgets (alias), entities->features (relative), shared->pages:
  // all upward. widgets->features and entities->entities must stay clean.
  const expected = ['../features/e', '@/pages/i', '@/widgets/b'].sort();
  if (JSON.stringify(ids) !== JSON.stringify(expected)) {
    console.error(`self-test failed: expected ${expected}, got ${ids}`);
    process.exit(1);
  }
  rmSync(root, { recursive: true, force: true });
  console.log('fsd check self-test passed');
}

if (process.argv.includes('--self-test')) selfTest();
const { files, violations } = scan(SRC);
for (const v of violations) {
  console.log(`${v.file}: imports ${v.target} (${v.from} -> ${v.to})`);
}
console.log(`${files} slice files, ${violations.length} violations`);
process.exit(violations.length === 0 ? 0 : 1);

