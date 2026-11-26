#!/usr/bin/env node
// Downloads the official llama.cpp `llama-server` build for a target and
// puts it (with its shared libraries) in src-tauri/llama/, which is bundled
// as an app resource. Vulkan builds for Windows/Linux (Intel, AMD, NVIDIA),
// Metal builds for macOS.
//
//   node scripts/fetch-llama.mjs [target-triple]
//
// LLAMA_TAG pins the release (default below). GITHUB_TOKEN is used if set.
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync, chmodSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir, platform, arch } from 'node:os';
import { fileURLToPath } from 'node:url';

const TAG = process.env.LLAMA_TAG || 'b12791';
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'src-tauri', 'llama');

const hostTriple = () => {
  if (platform() === 'win32') return 'x86_64-pc-windows-msvc';
  if (platform() === 'darwin') return arch() === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  return 'x86_64-unknown-linux-gnu';
};
const target = process.argv[2] || hostTriple();
const patterns = {
  'x86_64-pc-windows-msvc': /-bin-win-vulkan-x64\.zip$/,
  'x86_64-unknown-linux-gnu': /-bin-ubuntu-vulkan-x64\.(zip|tar\.gz)$/,
  'aarch64-apple-darwin': /-bin-macos-arm64\.(zip|tar\.gz)$/,
  'x86_64-apple-darwin': /-bin-macos-x64\.(zip|tar\.gz)$/,
};
const pattern = patterns[target];
if (!pattern) throw new Error(`No llama.cpp build for ${target}`);

const headers = { 'User-Agent': 'vocaflow-build', Accept: 'application/vnd.github+json' };
if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
const rel = await (await fetch(`https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/${TAG}`, { headers })).json();
const asset = (rel.assets || []).find((a) => pattern.test(a.name));
if (!asset) throw new Error(`No asset matching ${pattern} in llama.cpp ${TAG}: ${(rel.assets || []).map((a) => a.name).join(', ')}`);
console.log(`llama.cpp ${TAG}: ${asset.name}`);

const work = join(tmpdir(), `vf-llama-${Date.now()}`);
mkdirSync(work, { recursive: true });
const archive = join(work, asset.name);
const res = await fetch(asset.browser_download_url, { headers: { 'User-Agent': 'vocaflow-build' } });
if (!res.ok) throw new Error(`download failed: ${res.status}`);
writeFileSync(archive, Buffer.from(await res.arrayBuffer()));
const ex = join(work, 'x');
mkdirSync(ex);
if (asset.name.endsWith('.zip') && platform() === 'linux') execFileSync('unzip', ['-q', archive, '-d', ex]);
else execFileSync('tar', ['-xf', archive, '-C', ex]);

// Find the folder that holds llama-server and keep only it and its libraries.
const exe = target.includes('windows') ? 'llama-server.exe' : 'llama-server';
const find = (d) => {
  for (const f of readdirSync(d)) {
    const p = join(d, f);
    if (f === exe) return d;
    if (statSync(p).isDirectory()) {
      const r = find(p);
      if (r) return r;
    }
  }
  return null;
};
const binDir = find(ex);
if (!binDir) throw new Error('llama-server not found in archive');
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const keep = (f) => f === exe || /\.(dll|so(\.\d+)*|dylib|metallib)$/.test(f) || /\.so\.\d/.test(f);
for (const f of readdirSync(binDir)) {
  if (keep(f)) copyFileSync(join(binDir, f), join(outDir, f));
}
if (!target.includes('windows')) chmodSync(join(outDir, exe), 0o755);
writeFileSync(join(outDir, '.gitkeep'), '');

// Windows: ship the Khronos Vulkan loader next to the app. With no GPU
// driver installed, Vulkan then simply reports no device and everything
// runs on the CPU, instead of the app failing to start.
if (target.includes('windows')) {
  const sdk = process.env.VULKAN_SDK;
  const candidates = [
    sdk && join(sdk, 'Bin', 'vulkan-1.dll'),
    sdk && join(sdk, 'Runtime', 'x64', 'vulkan-1.dll'),
    'C:\\Windows\\System32\\vulkan-1.dll',
  ].filter(Boolean);
  const loader = candidates.find((p) => existsSync(p));
  if (!loader) throw new Error('vulkan-1.dll not found; install the Vulkan SDK/runtime');
  const vk = join(root, 'src-tauri', 'vulkan');
  mkdirSync(vk, { recursive: true });
  copyFileSync(loader, join(vk, 'vulkan-1.dll'));
  if (!existsSync(join(outDir, 'vulkan-1.dll'))) copyFileSync(loader, join(outDir, 'vulkan-1.dll'));
  console.log(`bundled ${loader}`);
}
console.log(`llama-server ready in ${outDir}: ${readdirSync(outDir).join(', ')}`);
