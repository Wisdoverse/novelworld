// Strict Feature-Sliced Design architecture gate.
//
// Checks every TypeScript source dependency using the TypeScript AST. Besides
// layer direction, sliced layers must be consumed through their public root,
// and slices in the same layer may not depend on one another.
// Run: node scripts/check-fsd.mjs [--self-test]
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, normalize, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const SRC = resolve(fileURLToPath(new URL('..', import.meta.url)), 'src');
const LAYERS = ['app', 'pages', 'widgets', 'features', 'entities', 'shared'];
const SLICED_LAYERS = new Set(['pages', 'widgets', 'features', 'entities']);
const RANK = new Map(LAYERS.map((layer, index) => [layer, index]));
const SOURCE_FILE = /\.(?:ts|tsx)$/;
const VITEST_MODULE_METHODS = new Set([
  'doMock',
  'doUnmock',
  'importActual',
  'importMock',
  'mock',
  'unmock',
]);
const JEST_MODULE_METHODS = new Set([
  'deepUnmock',
  'doMock',
  'dontMock',
  'mock',
  'requireActual',
  'requireMock',
  'setMock',
  'unmock',
]);

function pathParts(path) {
  return path.split(/[\\/]/).filter(Boolean);
}

function isInside(root, path) {
  const rel = relative(root, path);
  return rel === '' || (!rel.startsWith('..' + sep) && rel !== '..');
}

function locationOf(path, src) {
  if (!isInside(src, path)) return null;
  const rel = relative(src, path);
  const parts = pathParts(rel);
  const layer = LAYERS.includes(parts[0]) ? parts[0] : null;
  const slice = layer !== null && SLICED_LAYERS.has(layer) && parts.length > 1
    ? parts[1]
    : null;
  return { rel, parts, layer, slice };
}

function resolveLocalTarget(fromFile, specifier, src) {
  let path;
  if (specifier.startsWith('@/')) {
    path = resolve(src, ...specifier.slice(2).split('/'));
  } else if (specifier.startsWith('.')) {
    path = resolve(dirname(fromFile), specifier);
  } else {
    return null;
  }
  if (!isInside(src, path)) return null;
  return { path, location: locationOf(path, src) };
}

function staticSpecifier(node) {
  return node !== undefined && ts.isStringLiteralLike(node) ? node.text : null;
}

// Collect module edges from all syntax forms that can hide an architectural
// dependency. Tests are ordinary .ts/.tsx inputs, so mocks are checked too.
function moduleReferences(sourceFile) {
  const references = [];
  const add = (node, kind) => {
    const specifier = staticSpecifier(node);
    if (specifier !== null) references.push({ specifier, kind });
  };

  const visit = (node) => {
    if (ts.isImportDeclaration(node)) {
      add(node.moduleSpecifier, node.importClause?.isTypeOnly ? 'type-import' : 'import');
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier !== undefined) {
      add(node.moduleSpecifier, node.isTypeOnly ? 'type-export' : 'export');
    } else if (
      ts.isImportEqualsDeclaration(node) &&
      ts.isExternalModuleReference(node.moduleReference)
    ) {
      add(node.moduleReference.expression, 'require');
    } else if (ts.isImportTypeNode(node)) {
      const argument = ts.isLiteralTypeNode(node.argument) ? node.argument.literal : undefined;
      add(argument, 'import-type');
    } else if (ts.isCallExpression(node)) {
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        add(node.arguments[0], 'dynamic-import');
      } else if (ts.isIdentifier(node.expression) && node.expression.text === 'require') {
        add(node.arguments[0], 'require');
      } else {
        let namespace = null;
        let method = null;
        if (
          ts.isPropertyAccessExpression(node.expression) &&
          ts.isIdentifier(node.expression.expression)
        ) {
          namespace = node.expression.expression.text;
          method = node.expression.name.text;
        } else if (
          ts.isElementAccessExpression(node.expression) &&
          ts.isIdentifier(node.expression.expression) &&
          ts.isStringLiteralLike(node.expression.argumentExpression)
        ) {
          namespace = node.expression.expression.text;
          method = node.expression.argumentExpression.text;
        }
        if (
          (namespace === 'vi' && VITEST_MODULE_METHODS.has(method)) ||
          (namespace === 'jest' && JEST_MODULE_METHODS.has(method))
        ) {
          add(node.arguments[0], 'test-module');
        }
      }
    }
    ts.forEachChild(node, visit);
  };

  visit(sourceFile);
  return references;
}

function publicApiExists(sliceRoot) {
  return existsSync(join(sliceRoot, 'index.ts')) || existsSync(join(sliceRoot, 'index.tsx'));
}

function violation(category, dependency, source, target, detail) {
  return {
    category,
    file: source.rel,
    kind: dependency.kind,
    specifier: dependency.specifier,
    from: source.layer ?? 'src-root',
    to: target.layer ?? 'src-root',
    detail,
  };
}

function checkDependency(file, dependency, src) {
  const resolved = resolveLocalTarget(file, dependency.specifier, src);
  if (resolved === null) return null;

  const source = locationOf(file, src);
  const target = resolved.location;
  if (source === null || target === null) return null;

  // The only source-root composition boundary is an entrypoint importing app.
  // app and shared are intentionally unsliced; deep imports within/to shared
  // are permitted, while every sliced target is public-API guarded below.
  if (source.layer === null) {
    if (target.layer === 'app') return null;
    return violation(
      'root-boundary', dependency, source, target,
      'src-root entrypoints may import only app',
    );
  }

  if (target.layer === null) {
    return violation(
      'unlayered-target', dependency, source, target,
      'layered source may not depend on a src-root module',
    );
  }

  if (RANK.get(target.layer) < RANK.get(source.layer)) {
    return violation(
      'layer-direction', dependency, source, target,
      `${source.layer} may not import upward from ${target.layer}`,
    );
  }

  if (
    source.layer === target.layer &&
    SLICED_LAYERS.has(source.layer) &&
    source.slice !== target.slice
  ) {
    return violation(
      'same-layer-cross-slice', dependency, source, target,
      `${source.layer}/${source.slice} may not import ${target.layer}/${target.slice}`,
    );
  }

  const sameSlice = source.layer === target.layer && source.slice === target.slice;
  if (
    sameSlice &&
    SLICED_LAYERS.has(source.layer) &&
    dependency.specifier.startsWith('@/')
  ) {
    return violation(
      'same-slice-alias', dependency, source, target,
      `${source.layer}/${source.slice} must use a relative path for its own internals`,
    );
  }
  if (SLICED_LAYERS.has(target.layer) && !sameSlice) {
    if (target.slice === null) {
      return violation(
        'invalid-slice-target', dependency, source, target,
        `imports into ${target.layer} must name a slice`,
      );
    }

    const sliceRoot = resolve(src, target.layer, target.slice);
    if (normalize(resolved.path) !== normalize(sliceRoot)) {
      return violation(
        'public-api-bypass', dependency, source, target,
        `use @/${target.layer}/${target.slice}`,
      );
    }
    if (!publicApiExists(sliceRoot)) {
      return violation(
        'missing-public-api', dependency, source, target,
        `${target.layer}/${target.slice} needs index.ts or index.tsx`,
      );
    }
  }

  return null;
}

function scriptKind(file) {
  return file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
}

function sourceFiles(src) {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) walk(path);
      else if (SOURCE_FILE.test(entry)) files.push(path);
    }
  };
  walk(src);
  return files.sort();
}

function scan(src) {
  const files = sourceFiles(src);
  const violations = [];
  let dependencies = 0;
  for (const file of files) {
    const sourceFile = ts.createSourceFile(
      file,
      readFileSync(file, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      scriptKind(file),
    );
    for (const dependency of moduleReferences(sourceFile)) {
      dependencies++;
      const finding = checkDependency(file, dependency, src);
      if (finding !== null) violations.push(finding);
    }
  }
  return { files: files.length, dependencies, violations };
}

function categorySummary(violations) {
  const counts = new Map();
  for (const item of violations) {
    counts.set(item.category, (counts.get(item.category) ?? 0) + 1);
  }
  if (counts.size === 0) return 'none';
  return [...counts.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([category, count]) => `${category}=${count}`)
    .join(', ');
}

function selfTest() {
  const root = mkdtempSync(join(tmpdir(), 'strict-fsd-check-'));
  const src = join(root, 'src');
  const write = (rel, content) => {
    const path = join(src, rel);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content);
  };

  try {
    // Public APIs and allowed dependencies.
    write('app/App.tsx', [
      "import Home from '@/pages/home';",
      "import { client } from '@/shared/api/client';",
      'export const App = () => <Home client={client} />;',
    ].join('\n'));
    write('main.tsx', "import { App } from './app/App'; export { App };");
    write('shared/api/client.ts', 'export const client = {};');
    write('pages/home/index.ts', "export { default } from './ui/HomePage';");
    write('pages/home/ui/HomePage.tsx', [
      "import type { CardProps } from '@/widgets/card';",
      "import { search } from '@/features/search';",
      "import type { Book } from '@/entities/book';",
      "import { client } from '@/shared/api/client';",
      'export default function Home(_props: { client: typeof client } & CardProps & Book) { return search(); }',
    ].join('\n'));
    write('widgets/card/index.ts', "export type { CardProps } from './ui/Card';");
    write('widgets/card/ui/Card.ts', 'export type CardProps = {};');
    write('features/search/index.ts', "export { search } from './model/search';");
    write('features/search/model/search.ts', [
      "import { helper } from '../lib/helper';",
      "import type { Book } from '@/entities/book';",
      'export const search = (_book?: Book) => helper;',
    ].join('\n'));
    write('features/search/lib/helper.ts', 'export const helper = null;');
    write('features/search/model/self-alias.ts', "export { helper } from '@/features/search/lib/helper';");
    write('features/account/index.ts', "export const account = 'account';");
    write('entities/book/index.ts', "export type { Book } from './model/book';");
    write('entities/book/model/book.ts', 'export type Book = {};');

    // One fixture per rule/syntax path. Every edge below must be rejected.
    write('entities/book/model/up.ts', "import type { search } from '@/features/search'; export type X = typeof search;");
    write('entities/book/model/up-relative.ts', "import { search } from '../../../features/search'; export { search };");
    write('pages/home/ui/deep-import.ts', "import type { CardProps } from '@/widgets/card/ui/Card'; export type X = CardProps;");
    write('features/search/model/cross-slice.ts', "import { account } from '@/features/account'; export { account };");
    write('features/missing/model/value.ts', 'export const value = 1;');
    write('pages/home/ui/missing-api.ts', "import { value } from '@/features/missing'; export { value };");
    write('test-setup.ts', "vi.mock('@/features/search');");
    write('app/export.ts', "export { default } from '@/pages/home/ui/HomePage';");
    write('app/dynamic.ts', "export const load = () => import('@/features/search/model/search');");
    write('app/vi-mock.test.ts', "vi.mock('@/features/search/model/search');");
    write('app/jest-mock.test.ts', "jest.mock('@/features/search/model/search');");
    write('app/vi-do-mock.test.ts', "vi.doMock('@/features/search/model/search');");
    write('app/vi-import-actual.test.ts', "vi.importActual('@/features/search/model/search');");
    write('app/vi-import-mock.test.ts', "vi.importMock('@/features/search/model/search');");
    write('app/vi-unmock.test.ts', "vi.unmock('@/features/search/model/search');");
    write('app/vi-do-unmock.test.ts', "vi.doUnmock('@/features/search/model/search');");
    write('app/vi-element-access.test.ts', "vi['doMock']('@/features/search/model/search');");
    write('app/jest-do-mock.test.ts', "jest.doMock('@/features/search/model/search');");
    write('app/jest-unmock.test.ts', "jest.unmock('@/features/search/model/search');");
    write('app/jest-dont-mock.test.ts', "jest.dontMock('@/features/search/model/search');");
    write('app/jest-deep-unmock.test.ts', "jest.deepUnmock('@/features/search/model/search');");
    write('app/jest-require-actual.test.ts', "jest.requireActual('@/features/search/model/search');");
    write('app/jest-require-mock.test.ts', "jest.requireMock('@/features/search/model/search');");
    write('app/jest-set-mock.test.ts', "jest.setMock('@/features/search/model/search', {});");
    write('app/require.test.ts', "const search = require('@/features/search/model/search'); export { search };");
    write('app/import-type.ts', "export type Search = typeof import('@/features/search/model/search').search;");
    write('app/import-equals.ts', "import Search = require('@/features/search/model/search'); export { Search };");
    write('app/relative-deep.ts', "export { search } from '../features/search/model/search';");

    const result = scan(src);
    const actual = result.violations
      .map((item) => `${item.category}|${item.kind}|${item.specifier}`)
      .sort();
    const expected = [
      "layer-direction|type-import|@/features/search",
      'layer-direction|import|../../../features/search',
      'public-api-bypass|type-import|@/widgets/card/ui/Card',
      'same-layer-cross-slice|import|@/features/account',
      'missing-public-api|import|@/features/missing',
      'root-boundary|test-module|@/features/search',
      'public-api-bypass|export|@/pages/home/ui/HomePage',
      'public-api-bypass|dynamic-import|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|test-module|@/features/search/model/search',
      'public-api-bypass|require|@/features/search/model/search',
      'public-api-bypass|import-type|@/features/search/model/search',
      'public-api-bypass|require|@/features/search/model/search',
      'public-api-bypass|export|../features/search/model/search',
      'same-slice-alias|export|@/features/search/lib/helper',
    ].sort();

    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      console.error('FSD self-test failed.');
      console.error('expected: ' + JSON.stringify(expected, null, 2));
      console.error('actual:   ' + JSON.stringify(actual, null, 2));
      return false;
    }
    console.log(
      `FSD self-test: ${result.files} files, ${result.dependencies} dependency edges, ` +
      `${result.violations.length} expected violations (${categorySummary(result.violations)})`,
    );
    return true;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

if (process.argv.includes('--self-test') && !selfTest()) process.exit(1);

const result = scan(SRC);
for (const item of result.violations) {
  console.log(
    `[${item.category}] ${item.file}: ${item.kind} ${item.specifier} ` +
    `(${item.from} -> ${item.to}) - ${item.detail}`,
  );
}
console.log(
  `FSD scan: ${result.files} files, ${result.dependencies} dependency edges, ` +
  `${result.violations.length} violations (${categorySummary(result.violations)})`,
);
process.exit(result.violations.length === 0 ? 0 : 1);
