import { afterEach, expect, test } from 'bun:test';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const temporary = [];
afterEach(() => { for (const path of temporary.splice(0)) rmSync(path, { recursive: true, force: true }); });

function check(headers) {
  const dir = mkdtempSync(join(tmpdir(), 'mailx-sdk-check-'));
  temporary.push(dir);
  const app = join(dir, 'MailX with spaces.app');
  const binaries = join(app, 'Contents', 'MacOS');
  const tools = join(dir, 'tools');
  mkdirSync(binaries, { recursive: true });
  mkdirSync(tools);
  for (const [index, header] of headers.entries()) {
    const binary = join(binaries, `binary-${index}`);
    writeFileSync(binary, '#!/bin/sh\n');
    chmodSync(binary, 0o755);
    writeFileSync(`${binary}.headers`, header);
  }
  writeFileSync(join(tools, 'otool'), '#!/bin/sh\ncat "$2.headers"\n');
  chmodSync(join(tools, 'otool'), 0o755);
  return spawnSync('bash', [resolve(import.meta.dirname, 'check-macos-sdk.sh'), app], {
    env: { ...process.env, PATH: `${tools}:${process.env.PATH}` }, encoding: 'utf8',
  });
}

const modern = (sdk) => `Load command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 1\n    minos 11.0\n      sdk ${sdk}\n   ntools 1\n`;
const legacy = (sdk) => `Load command 9\n      cmd LC_VERSION_MIN_MACOSX\n  cmdsize 16\n  version 10.13\n      sdk ${sdk}\n`;

test('accepts modern and Intel legacy SDK metadata without confusing deployment target with SDK', () => {
  const result = check([modern('26.0'), legacy('27.0')]);
  expect(result.status).toBe(0);
  expect(result.stdout).toContain('SDK 26.0');
  expect(result.stdout).toContain('SDK 27.0');
});

test('rejects older SDKs in either header format', () => {
  for (const header of [modern('15.5'), legacy('15.5')]) {
    expect(check([header]).status).toBe(1);
  }
});

test('checks every architecture and every executable', () => {
  expect(check([modern('26.0') + legacy('15.5')]).status).toBe(1);
  expect(check([modern('26.0'), legacy('15.5')]).status).toBe(1);
});

test('rejects missing or malformed metadata and empty app bundles', () => {
  expect(check(['cmd LC_UUID\n cmdsize 24\n']).status).toBe(1);
  expect(check([modern('unknown')]).status).toBe(1);
  expect(check([]).status).toBe(1);
});
