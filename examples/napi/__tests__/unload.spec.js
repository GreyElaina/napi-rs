// use the commonjs syntax to prevent compiler from transpiling the module syntax

import { createRequire } from 'node:module'
import * as path from 'node:path'

import test from 'ava'

const require = createRequire(import.meta.url)
const __dirname = path.dirname(new URL(import.meta.url).pathname)

const PLATFORM_MAP = {
  darwin: 'darwin',
  linux: 'linux',
  win32: 'win32',
  freebsd: 'freebsd',
}
const ARCH_MAP = { x64: 'x64', arm64: 'arm64', ia32: 'ia32', arm: 'arm' }

const platform = PLATFORM_MAP[process.platform] ?? process.platform
const arch = ARCH_MAP[process.arch] ?? process.arch

let abi
if (process.platform === 'linux') {
  if (process.arch === 'arm') {
    abi = 'gnueabihf'
  } else if (process.report?.getReport?.()?.header.glibcVersionRuntime) {
    abi = 'gnu'
  } else {
    abi = 'musl'
  }
} else if (process.platform === 'win32') {
  abi = 'msvc'
}

const binaryName = abi
  ? `example.${platform}-${arch}-${abi}.node`
  : `example.${platform}-${arch}.node`

test('unload module', (t) => {
  const { add } = require(`../${binaryName}`)
  t.is(add(1, 2), 3)
  delete require.cache[require.resolve(`../${binaryName}`)]
  const { add: add2 } = require(`../${binaryName}`)
  t.is(add2(1, 2), 3)
})

test('load module multi times', (t) => {
  if (process.platform === 'win32') {
    t.pass()
    return
  }
  const { add } = require(`../${binaryName}`)
  t.is(add(1, 2), 3)
  const { add: add2 } = require(
    path.toNamespacedPath(path.join(__dirname, `../${binaryName}`)),
  )
  t.is(add2(1, 2), 3)
})
