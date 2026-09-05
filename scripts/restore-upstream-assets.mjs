import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

const upstreamCommit = "ef0d42bff096403b6331e2fded60c928a31c6782";
const assets = [
  {
    path: "docs/screenshots/home-dashboard.png",
    sha256: "007747a55d4eca9784b077cd0004ff70af1a4b7ba426bcd424fe488f8aea0d5a",
  },
  {
    path: "docs/screenshots/post-game.png",
    sha256: "c8104e11a1752e59bb362c6afd3c7d48082bac3b0dcb64185d6adb4db82aef88",
  },
  {
    path: "src-tauri/resources/Assets.car",
    sha256: "471963dc7259a45b0347c7422616f99f8853b2a636001fc7ba300a0fc3f24d01",
  },
];

function checksum(content) {
  return createHash("sha256").update(content).digest("hex");
}

async function matches(asset) {
  try {
    return checksum(await readFile(asset.path)) === asset.sha256;
  } catch {
    return false;
  }
}

for (const asset of assets) {
  if (await matches(asset)) continue;

  const url = `https://raw.githubusercontent.com/ishtartec/query-lol-desktop/${upstreamCommit}/${asset.path}`;
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to restore ${asset.path}: HTTP ${response.status}`);
  }

  const content = Buffer.from(await response.arrayBuffer());
  const actual = checksum(content);
  if (actual !== asset.sha256) {
    throw new Error(`Checksum mismatch for ${asset.path}: expected ${asset.sha256}, got ${actual}`);
  }

  await mkdir(dirname(asset.path), { recursive: true });
  await writeFile(asset.path, content);
  console.log(`Restored ${asset.path}`);
}
