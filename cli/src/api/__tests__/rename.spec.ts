import { existsSync } from 'node:fs'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import ava, { type TestFn } from 'ava'
import { load as yamlLoad } from 'js-yaml'

import { renameProject } from '../rename.js'

const test = ava as TestFn<{
  tmpDir: string
}>

test.beforeEach((t) => {
  const timestamp = Date.now()
  const random = Math.random().toString(36).substring(7)
  t.context = {
    tmpDir: join(
      tmpdir(),
      'napi-rs-test',
      `rename-project-${timestamp}-${random}`,
    ),
  }
})

test.afterEach.always(async (t) => {
  if (existsSync(t.context.tmpDir)) {
    await rm(t.context.tmpDir, { recursive: true, force: true })
  }
})

async function createFixtureProject(
  cwd: string,
  options: {
    packageJson: Record<string, unknown>
    cargoPackageName: string
    configPath?: string
    configData?: Record<string, unknown>
  },
) {
  await mkdir(join(cwd, '.github', 'workflows'), { recursive: true })

  await writeFile(
    join(cwd, 'package.json'),
    `${JSON.stringify(options.packageJson, null, 2)}\n`,
  )
  await writeFile(
    join(cwd, 'Cargo.toml'),
    `[package]\nname = "${options.cargoPackageName}"\n`,
  )
  await writeFile(
    join(cwd, '.github', 'workflows', 'CI.yml'),
    'env:\n  APP_NAME: foo\njobs:\n  build:\n    runs-on: ubuntu-latest\n',
  )

  if (options.configPath && options.configData) {
    await writeFile(
      join(cwd, options.configPath),
      `${JSON.stringify(options.configData, null, 2)}\n`,
    )
  }
}

test('omitting binaryName keeps package binary references', async (t) => {
  const projectPath = join(t.context.tmpDir, 'artifact-rename')

  await createFixtureProject(projectPath, {
    packageJson: {
      name: 'original',
      napi: {
        binaryName: 'foo',
        packageName: '@scope/original',
      },
    },
    cargoPackageName: 'foo',
  })

  await renameProject({
    cwd: projectPath,
    name: 'renamed',
  })

  const packageJson = JSON.parse(
    await readFile(join(projectPath, 'package.json'), 'utf8'),
  )
  const cargoToml = await readFile(join(projectPath, 'Cargo.toml'), 'utf8')
  const ciYaml = yamlLoad(
    await readFile(join(projectPath, '.github', 'workflows', 'CI.yml'), 'utf8'),
  ) as any

  t.is(packageJson.name, 'renamed')
  t.is(packageJson.napi.binaryName, 'foo')
  t.is(packageJson.napi.packageName, '@scope/original')
  t.true(cargoToml.includes('name = "foo"'))
  t.is(ciYaml.env.APP_NAME, 'foo')
})

test('omitting binaryName preserves separated napi config fields', async (t) => {
  const projectPath = join(t.context.tmpDir, 'config-rename')

  await createFixtureProject(projectPath, {
    packageJson: {
      name: 'original',
    },
    cargoPackageName: 'foo',
    configPath: 'napi.json',
    configData: {
      binaryName: 'foo',
      packageName: '@scope/original',
    },
  })

  await renameProject({
    cwd: projectPath,
    configPath: 'napi.json',
    name: 'renamed',
  })

  const config = JSON.parse(
    await readFile(join(projectPath, 'napi.json'), 'utf8'),
  )

  t.is(config.binaryName, 'foo')
  t.is(config.packageName, '@scope/original')
})

test('repository updates package.json when provided', async (t) => {
  const projectPath = join(t.context.tmpDir, 'repository-rename')

  await createFixtureProject(projectPath, {
    packageJson: {
      name: 'original',
      repository: {
        type: 'git',
        url: 'https://example.com/old.git',
      },
      napi: {
        binaryName: 'foo',
        packageName: '@scope/original',
      },
    },
    cargoPackageName: 'foo',
  })

  await renameProject({
    cwd: projectPath,
    name: 'renamed',
    repository: 'https://example.com/new.git',
  })

  const packageJson = JSON.parse(
    await readFile(join(projectPath, 'package.json'), 'utf8'),
  )

  t.is(packageJson.name, 'renamed')
  t.is(packageJson.repository.url, 'https://example.com/new.git')
  t.is(packageJson.repository.type, 'git')
})
