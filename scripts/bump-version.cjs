const fs = require('fs');

const targetVersion = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(targetVersion || '')) {
  throw new Error('Usage: node scripts/bump-version.cjs <major.minor.patch>');
}

// 1. package.json
const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
pkg.version = targetVersion;
fs.writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n', 'utf8');

// 2. package-lock.json
if (fs.existsSync('package-lock.json')) {
  const lock = JSON.parse(fs.readFileSync('package-lock.json', 'utf8'));
  lock.version = targetVersion;
  if (lock.packages && lock.packages['']) {
    lock.packages[''].version = targetVersion;
  }
  fs.writeFileSync('package-lock.json', JSON.stringify(lock, null, 2) + '\n', 'utf8');
}

// 3. tauri.conf.json
const tauriConf = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
tauriConf.version = targetVersion;
fs.writeFileSync('src-tauri/tauri.conf.json', JSON.stringify(tauriConf, null, 2) + '\n', 'utf8');

// 4. Cargo.toml
let cargoToml = fs.readFileSync('src-tauri/Cargo.toml', 'utf8');
cargoToml = cargoToml.replace(/^version\s*=\s*"[^"]+"/m, `version = "${targetVersion}"`);
fs.writeFileSync('src-tauri/Cargo.toml', cargoToml, 'utf8');

// Keep the app package in Cargo.lock consistent without changing dependency versions.
const cargoLockPath = 'src-tauri/Cargo.lock';
const cargoLock = fs.readFileSync(cargoLockPath, 'utf8').replace(
  /(\[\[package\]\]\s+name = "otium_downloader"\s+version = ")[^"]+("[\s\S]*?)(?=\[\[package\]\]|$)/,
  (_, prefix, rest) => `${prefix}${targetVersion}${rest}`
);
fs.writeFileSync(cargoLockPath, cargoLock, 'utf8');

// 5. Visible version and browser integration
const headerPath = 'src/components/Header.tsx';
fs.writeFileSync(headerPath, fs.readFileSync(headerPath, 'utf8').replace(/CURRENT_VERSION\s*=\s*'[^']+'/, `CURRENT_VERSION = '${targetVersion}'`));
const manifestPath = 'browser-extension/manifest.json';
const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
manifest.version = targetVersion;
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');

console.log(`Bumped ARGON to ${targetVersion}`);
