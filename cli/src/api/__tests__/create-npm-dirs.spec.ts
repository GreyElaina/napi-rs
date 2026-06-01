import { existsSync } from 'node:fs'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import ava, { type TestFn } from 'ava'

import { createNpmDirs } from '../create-npm-dirs.js'

const test = ava as TestFn<{
  tmpDir: string
  packageJsonPath: string
  npmConfigRegistry?: string
}>

test.beforeEach(async (t) => {
  // Create a unique temp directory for tests
  const timestamp = Date.now()
  const random = Math.random().toString(36).substring(7)
  const tmpDir = join(
    tmpdir(),
    'napi-rs-test',
    `create-npm-dirs-${timestamp}-${random}`,
  )
  const packageJsonPath = join(tmpDir, 'package.json')

  // Create the directory
  await mkdir(tmpDir, { recursive: true })

  t.context = {
    tmpDir,
    packageJsonPath,
    npmConfigRegistry: process.env.npm_config_registry,
  }
})

test.afterEach.always(async (t) => {
  if (t.context.npmConfigRegistry === undefined) {
    delete process.env.npm_config_registry
  } else {
    process.env.npm_config_registry = t.context.npmConfigRegistry
  }

  // Clean up any created directories
  if (existsSync(t.context.tmpDir)) {
    await rm(t.context.tmpDir, { recursive: true, force: true })
  }
})

test('should omit exports fields from publishConfig in scoped packages', async (t) => {
  const { tmpDir, packageJsonPath } = t.context

  // Create a package.json with publishConfig that includes exports field
  const packageJson = {
    name: 'test-package',
    version: '1.0.0',
    publishConfig: {
      registry: 'https://custom-registry.com',
      access: 'public',
      exports: {
        '.': './dist/index.js',
        './package.json': './package.json',
      },
      tag: 'beta',
    },
    napi: {
      binaryName: 'test-package',
      targets: ['x86_64-unknown-linux-gnu'],
    },
  }

  await writeFile(packageJsonPath, JSON.stringify(packageJson, null, 2))

  await createNpmDirs({
    cwd: tmpDir,
    packageJsonPath: 'package.json',
  })

  // Check that the scoped package directory was created
  const scopedDir = join(tmpDir, 'npm', 'linux-x64-gnu')
  t.true(existsSync(scopedDir))

  // Read the generated package.json for the scoped package
  const scopedPackageJsonPath = join(scopedDir, 'package.json')
  t.true(existsSync(scopedPackageJsonPath))

  const scopedPackageJson = JSON.parse(
    await readFile(scopedPackageJsonPath, 'utf-8'),
  )

  // Verify that publishConfig only contains registry and access, not exports
  t.truthy(scopedPackageJson.publishConfig)
  t.is(scopedPackageJson.publishConfig.registry, 'https://custom-registry.com')
  t.is(scopedPackageJson.publishConfig.access, 'public')
  t.is(scopedPackageJson.publishConfig.exports, undefined)
  t.is(scopedPackageJson.publishConfig.tag, undefined)
})

test('should handle package without publishConfig', async (t) => {
  const { tmpDir, packageJsonPath } = t.context

  // Create a package.json without publishConfig
  const packageJson = {
    name: 'test-package-no-config',
    version: '1.0.0',
    napi: {
      binaryName: 'test-package-no-config',
      targets: ['x86_64-unknown-linux-gnu'],
    },
  }

  await writeFile(packageJsonPath, JSON.stringify(packageJson, null, 2))

  await createNpmDirs({
    cwd: tmpDir,
    packageJsonPath: 'package.json',
  })

  // Check that the scoped package directory was created
  const scopedDir = join(tmpDir, 'npm', 'linux-x64-gnu')
  t.true(existsSync(scopedDir))

  // Read the generated package.json for the scoped package
  const scopedPackageJsonPath = join(scopedDir, 'package.json')
  const scopedPackageJson = JSON.parse(
    await readFile(scopedPackageJsonPath, 'utf-8'),
  )

  // Verify that publishConfig is not present when not in source
  t.is(scopedPackageJson.publishConfig, undefined)
})

test('should preserve only registry and access in publishConfig', async (t) => {
  const { tmpDir, packageJsonPath } = t.context

  // Create a package.json with minimal publishConfig
  const packageJson = {
    name: 'test-package-minimal',
    version: '1.0.0',
    publishConfig: {
      registry: 'https://npm.company.com',
      access: 'restricted',
    },
    napi: {
      binaryName: 'test-package-minimal',
      targets: ['aarch64-apple-darwin'],
    },
  }

  await writeFile(packageJsonPath, JSON.stringify(packageJson, null, 2))

  await createNpmDirs({
    cwd: tmpDir,
    packageJsonPath: 'package.json',
  })

  // Check that the scoped package directory was created
  const scopedDir = join(tmpDir, 'npm', 'darwin-arm64')
  t.true(existsSync(scopedDir))

  // Read the generated package.json for the scoped package
  const scopedPackageJsonPath = join(scopedDir, 'package.json')
  const scopedPackageJson = JSON.parse(
    await readFile(scopedPackageJsonPath, 'utf-8'),
  )

  // Verify that publishConfig contains exactly registry and access
  t.truthy(scopedPackageJson.publishConfig)
  t.is(scopedPackageJson.publishConfig.registry, 'https://npm.company.com')
  t.is(scopedPackageJson.publishConfig.access, 'restricted')
  t.is(Object.keys(scopedPackageJson.publishConfig).length, 2)
})
