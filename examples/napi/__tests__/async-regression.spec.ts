import test from 'ava'

import {
  regressionPromiseIfSpawnFails,
  shutdownRuntime,
} from '../index.cjs'

test.after(() => {
  shutdownRuntime()
})

test('spawn failure must reject promise instead of leaving it pending', async (t) => {
  const promise = regressionPromiseIfSpawnFails()
  const outcome = await Promise.race([
    promise.then(
      () => 'resolved' as const,
      () => 'rejected' as const,
    ),
    new Promise<'timeout'>((resolve) => {
      setTimeout(() => resolve('timeout'), 500)
    }),
  ])

  t.not(
    outcome,
    'timeout',
    'promise must settle when spawn_future fails after deferred creation',
  )
  t.is(outcome, 'rejected')
})
