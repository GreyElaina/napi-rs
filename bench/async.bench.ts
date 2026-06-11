import { cpus } from 'node:os'
import { createRequire } from 'node:module'

import { bench } from 'vitest'

const require = createRequire(import.meta.url)

const {
  benchBlocking,
  benchTokioFuture,
} = require('./index.node')

const buffer = Buffer.from('hello 🚀 rust!')

const ALL_THREADS = Array.from({ length: cpus().length })

bench('blocking promise', async () => {
  await Promise.all(ALL_THREADS.map(() => benchBlocking(buffer)))
})

bench('Tokio future to Promise', async () => {
  await Promise.all(ALL_THREADS.map(() => benchTokioFuture(buffer)))
})
