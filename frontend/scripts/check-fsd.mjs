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
// not cross into another recognized layer (same-slice imports, bare packages,
// non-layer directories).
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

function skipString(code, i) {
  const quote = code[i];
  i++;
  while (i < code.length) {
    if (code[i] === '\\') { i += 2; continue; }
    if (code[i] === quote) return i + 1;
    i++;
  }
  return i;
}

// Extract module specifiers from REAL import/export statements only: comments
// and string/template literals outside statements are skipped entirely, and a
// keyword must sit at a code boundary, so doc text and messages can never trip
// the gate. Handles: import x from, import {a} from, import * as x from,
// import type {a} from, import '..', import('..'), export {a} from,
// export * from. A quoted specifier is captured only after 'import' or after
// 'from' whose preceding word is not 'import' (prose like
// "see import from '@/widgets/b'" inside JSX text stays clean).
function importTargets(code) {
  const targets = [];
  const isKeyword = (pos, kw) =>
    code.startsWith(kw, pos) &&
    !/[A-Za-z0-9_$>}]/.test(code[pos - 1] ?? '') &&
    !/[A-Za-z0-9_$]/.test(code[pos + kw.length] ?? '');
  let i = 0;
  while (i < code.length) {
    const ch = code[i];
    const next = code[i + 1];
    if (ch === '/' && next === '/') {
      while (i < code.length && code[i] !== '\n') i++;
    } else if (ch === '/' && next === '*') {
      i += 2;
      while (i < code.length && !(code[i] === '*' && code[i + 1] === '/')) i++;
      i += 2;
    } else if (ch === '"' || ch === "'" || ch === '\u0060') {
      i = skipString(code, i);
    } else if (isKeyword(i, 'import') || isKeyword(i, 'export')) {
      // 'import from ...' / 'export from ...' is prose, not a statement.
      if (/^\s*from\b/.test(code.slice(i + 6))) {
        i++;
        continue;
      }
      i += 6;
      // Scan to the end of the statement, capturing quoted specifiers.
      let paren = 0;
      let last1 = 'import';
      let last2 = '';
      let word = '';
      while (i < code.length) {
        const c = code[i];
        if (c === '(') paren++;
        else if (c === ')' && paren > 0) paren--;
        else if ((c === ';' || c === '\n') && paren === 0) break;
        else if (/[A-Za-z0-9_$]/.test(c)) {
          word += c;
          i++;
          continue;
        } else {
          if (word) {
            last2 = last1;
            last1 = word;
            word = '';
          }
          if (c === '"' || c === "'" || c === '\u0060') {
            if (last1 === 'import' || (last1 === 'from' && last2 !== 'import')) {
              const start = i;
              i = skipString(code, i);
              targets.push(code.slice(start + 1, i - 1));
              continue;
            }
          }
        }
        i++;
      }
    } else {
      i++;
    }
  }
  return targets;
}

function checkFile(file, src, violations) {
  const rel = relative(src, file);
  const layer = layerOf(rel);
  if (layer === null) return; // src-root files (main.tsx, test-setup.ts)
  for (const target of importTargets(readFileSync(file, 'utf8'))) {
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
  const root = mkdtempSync(join(process.env.TMPDIR ?? '/tmp', 'fsd-check-'));
  const write = (rel, content) => {
    const path = join(root, rel);
    mkdirSync(path.slice(0, path.lastIndexOf(sep)), { recursive: true });
    writeFileSync(path, content);
  };
  try {
    write('src/features/a.ts', "import { x } from '@/widgets/b';");
    write('src/features/c.ts', "import { y } from '@/entities/d';");
    write('src/widgets/b.ts', "import { z } from '../features/e';");
    write('src/features/e.ts', 'export const z = 1;');
    write('src/entities/k.ts', "import { q } from '../features/e';");
    write('src/entities/d.ts', "import { g } from './g'; export const y = g;");
    write('src/entities/g.ts', 'export const g = 1;');
    write('src/shared/h.ts', "import { i } from '@/pages/i';");
    write('src/pages/i.ts', 'export const i = 1;');
    // Bypass classes: side-effect and dynamic imports are violations too.
    write('src/features/s.ts', "import '@/pages/side';");
    write('src/pages/side.ts', 'export const side = 1;');
    write('src/entities/dyn.ts', "export async function load() { return import('@/widgets/dyn'); }");
    write('src/widgets/dyn.ts', 'export const dyn = 1;');
    // Comments, strings, and JSX text naming upward specifiers must NOT trip
    // the gate.
    write('src/features/doc.ts', [
      "// TODO: stop importing from '@/widgets/b' someday",
      "const msg = \"never import from '@/pages/i'\";",
      "export const doc = <p>see import from '@/widgets/b' in the docs</p>;",
    ].join('\n'));
    const { violations } = scan(join(root, 'src'));
    const ids = violations.map((v) => v.target).sort();
    // Upward: alias, relative, side-effect, dynamic. Downward/same-layer and
    // comment/string/JSX mentions stay clean.
    const expected = ['../features/e', '@/pages/i', '@/pages/side', '@/widgets/b', '@/widgets/dyn'].sort();
    if (JSON.stringify(ids) !== JSON.stringify(expected)) {
      console.error('self-test failed: expected ' + JSON.stringify(expected) + ', got ' + JSON.stringify(ids));
      return false;
    }
    return true;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

if (process.argv.includes('--self-test') && !selfTest()) process.exit(1);
const { files, violations } = scan(SRC);
for (const v of violations) {
  console.log(v.file + ': imports ' + v.target + ' (' + v.from + ' -> ' + v.to + ')');
}
console.log(files + ' slice files, ' + violations.length + ' violations');
process.exit(violations.length === 0 ? 0 : 1);