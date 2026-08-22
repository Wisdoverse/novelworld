import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const [target, pg0Asset, expectedSha] = process.argv.slice(2);
if (!target || !pg0Asset || !/^[0-9a-f]{64}$/.test(expectedSha || '')) {
  throw new Error('usage: node tools/desktop/prepare.mjs <rust-target> <pg0-asset> <sha256>');
}

const windows = target.includes('windows');
const extension = windows ? '.exe' : '';
const sourceDir = join('target', target, 'release');
const outputDir = join('frontend', 'src-tauri', 'resources', 'bin');
const services = ['gateway', 'user-service', 'novel-service', 'agent-service', 'narrative-service'];

await mkdir(outputDir, { recursive: true });
for (const service of services) {
  const output = join(outputDir, `${service}${extension}`);
  await rm(output, { force: true });
  await copyFile(join(sourceDir, `${service}${extension}`), output);
  if (!windows) await chmod(output, 0o755);
}

const version = '0.15.1';
const response = await fetch(`https://github.com/vectorize-io/pg0/releases/download/v${version}/${pg0Asset}`);
if (!response.ok) throw new Error(`pg0 download failed: HTTP ${response.status}`);
const bytes = Buffer.from(await response.arrayBuffer());
const actualSha = createHash('sha256').update(bytes).digest('hex');
if (actualSha !== expectedSha) {
  throw new Error(`pg0 checksum mismatch: expected ${expectedSha}, got ${actualSha}`);
}

const pg0Output = join(outputDir, `pg0${extension}`);
await writeFile(pg0Output, bytes);
if (!windows) await chmod(pg0Output, 0o755);

console.log(`Prepared ${target} desktop runtime with pg0 ${version}.`);
