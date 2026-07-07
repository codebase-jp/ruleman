#!/usr/bin/env node
// Synchronizes a single version number across Cargo.toml and every npm/ package.json.
// Used by the release workflow right after a `vX.Y.Z` tag is pushed.
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const version = process.argv[2]
if (!version || !/^\d+\.\d+\.\d+(-.+)?$/.test(version)) {
  console.error('usage: node sync-version.mjs <semver-version>')
  process.exit(1)
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

function updateJson(relativePath, mutate) {
  const fullPath = path.join(root, relativePath)
  const pkg = JSON.parse(readFileSync(fullPath, 'utf8'))
  mutate(pkg)
  writeFileSync(fullPath, JSON.stringify(pkg, null, 2) + '\n')
}

const PLATFORM_DIRS = [
  'linux-x64-gnu',
  'linux-arm64-gnu',
  'darwin-x64',
  'darwin-arm64',
  'win32-x64-msvc',
]

for (const dir of PLATFORM_DIRS) {
  updateJson(`npm/platforms/${dir}/package.json`, (pkg) => {
    pkg.version = version
  })
}

updateJson('npm/ruleman/package.json', (pkg) => {
  pkg.version = version
  for (const dir of PLATFORM_DIRS) {
    pkg.optionalDependencies[`ruleman-${dir}`] = version
  }
})

const cargoTomlPath = path.join(root, 'Cargo.toml')
const cargoToml = readFileSync(cargoTomlPath, 'utf8')
writeFileSync(cargoTomlPath, cargoToml.replace(/^version = ".*"$/m, `version = "${version}"`))

console.log(`Synced version ${version} across Cargo.toml and npm/*/package.json`)
