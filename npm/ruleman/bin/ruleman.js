#!/usr/bin/env node
'use strict'

const { spawnSync } = require('node:child_process')

// Keyed by `${process.platform}-${process.arch}`.
const PLATFORMS = {
  'linux-x64': 'ruleman-linux-x64-gnu',
  'linux-arm64': 'ruleman-linux-arm64-gnu',
  'darwin-x64': 'ruleman-darwin-x64',
  'darwin-arm64': 'ruleman-darwin-arm64',
  'win32-x64': 'ruleman-win32-x64-msvc',
}

function resolveBinaryPath() {
  const key = `${process.platform}-${process.arch}`
  const pkgName = PLATFORMS[key]
  if (!pkgName) {
    throw new Error(
      `ruleman does not ship a prebuilt binary for "${key}".\n` +
        'Supported platforms: ' + Object.keys(PLATFORMS).join(', ') + '\n' +
        'See https://github.com/codebase-jp/ruleman#building-from-source to build from source.'
    )
  }

  const binaryName = process.platform === 'win32' ? 'ruleman.exe' : 'ruleman'

  try {
    return require.resolve(`${pkgName}/${binaryName}`)
  } catch {
    throw new Error(
      `ruleman could not find its optional dependency "${pkgName}".\n` +
        'Try reinstalling with npm, and make sure optionalDependencies are not being skipped ' +
        '(e.g. via --no-optional, or an npm/yarn/pnpm config that omits them).'
    )
  }
}

function main() {
  let binaryPath
  try {
    binaryPath = resolveBinaryPath()
  } catch (err) {
    console.error(err.message)
    process.exit(1)
  }

  const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: 'inherit' })

  if (result.error) {
    console.error(`ruleman failed to launch the native binary: ${result.error.message}`)
    process.exit(1)
  }

  process.exit(result.status === null ? 1 : result.status)
}

main()
