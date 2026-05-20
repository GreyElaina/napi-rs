import { join } from 'path'
import { fileURLToPath } from 'url'

import test from 'ava'

import { correctStringIdent, processTypeDef } from '../typegen.js'

test('should ident string correctly', (t) => {
  const input = `
  /**
   * should keep
   * class A {
   * foo = () => {}
   *   bar = () => {}
   * }
   */
  class A {
    foo() {
      a = b
    }

  bar = () => {

  }
      boz = 1
    }

  namespace B {
      namespace C {
  type D = A
      }
  }
`
  t.snapshot(correctStringIdent(input, 0), 'original ident is 0')
  t.snapshot(correctStringIdent(input, 2), 'original ident is 2')
})

test('should process type def correctly', async (t) => {
  const { dts, exports } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'napi_type_def',
    ),
    true,
  )

  t.snapshot(dts)
  t.true(exports.includes('CssStyleSheet'))
  t.false(exports.includes('CSSStyleSheet'))
})

test('should process type def with noConstEnum correctly', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'napi_type_def',
    ),
    false,
  )

  t.snapshot(dts)
})

// The next two tests use a minimal fixture (one numeric + one string
// enum) to keep snapshots small and focused on the flag's behavior.
const flagFixture = join(
  fileURLToPath(import.meta.url),
  '../',
  '__fixtures__',
  'runtime_string_enum_flag',
)

test('should process type def with noConstEnum and runtimeStringEnum correctly', async (t) => {
  const { dts } = await processTypeDef(flagFixture, false, true)

  t.snapshot(dts)
})

test('runtimeStringEnum is a no-op when constEnum is set', async (t) => {
  const { dts } = await processTypeDef(flagFixture, true, true)

  t.snapshot(dts)
})

test('resolves class parents within the same namespace', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'module_scoped_extends',
    ),
    true,
  )

  t.true(dts.includes('export class Child extends Base {'))
})

test('resolves class parents by original rust name', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'module_scoped_extends_alias',
    ),
    true,
  )

  t.true(dts.includes('export class Child extends PublicBase {'))
})

test('keeps native parent when iterator methods are merged', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'module_scoped_extends_iterator',
    ),
    true,
  )

  t.true(dts.includes('export class Child extends Base {'))
  t.true(
    dts.includes('next(value?: undefined): IteratorResult<number, void>'),
  )
})

test('constructible child inherits non-constructible native parent instance type', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'constructible_extends_non_constructible',
    ),
    true,
  )

  t.true(dts.includes('export interface Base {'))
  t.true(dts.includes('base(): number'))
  t.true(dts.includes('export const Base: {'))
  t.true(dts.includes('export class Child {'))
  t.true(dts.includes('child(): number'))
  t.true(dts.includes('export interface Child extends Base {}'))
  t.false(dts.includes('export class Child extends Base'))
  t.false(dts.includes('export declare class Base'))
})

test('non-constructible iterator class remains interface plus value', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'non_constructible_iterator_class',
    ),
    true,
  )

  t.true(
    dts.includes(
      'export interface IteratorBase extends Iterator<number, void, undefined> {',
    ),
  )
  t.true(dts.includes('tick(): number'))
  t.true(dts.includes('export declare const IteratorBase: {'))
  t.false(dts.includes('export declare class IteratorBase extends Iterator'))
})

test('factory-only iterator class remains non-constructible', async (t) => {
  const { dts } = await processTypeDef(
    join(
      fileURLToPath(import.meta.url),
      '../',
      '__fixtures__',
      'napi_type_def',
    ),
    true,
  )

  t.true(dts.includes('export interface Fib2 {'))
  t.true(dts.includes('[Symbol.iterator](): Iterator<number, void, number>'))
  t.true(dts.includes('create(seed: number): Fib2'))
  t.false(dts.includes('static create(seed: number): Fib2'))
  t.true(dts.includes('export declare const Fib2: {'))
  t.false(dts.includes('export declare class Fib2'))
})

test('rejects unresolved native parent', async (t) => {
  await t.throwsAsync(
    processTypeDef(
      join(
        fileURLToPath(import.meta.url),
        '../',
        '__fixtures__',
        'module_scoped_extends_missing',
      ),
      true,
    ),
    {
      message:
        'Native class parent MissingBase for Child is not present in the same TypeScript generation unit',
    },
  )
})
