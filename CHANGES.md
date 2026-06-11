# Fork Changes

Diverged from upstream napi-rs at [`e9c50bb4`](https://github.com/napi-rs/napi-rs/commit/e9c50bb4) (v3 release).

33 commits, 98 files changed (+13k/−11k lines). The core theme is **lifetime-safe bindings** — encoding JavaScript value scoping rules into Rust's type system so that use-after-free, dangling ref, and env-mismatch bugs become compile errors.

---

## 0. Breaking Change: WASM/WASI Support Removed

This fork no longer supports WASM/WASI builds. The supported artifact model is native Node-API addons only.

Removed support includes:

- `wasm32-*` targets, including `wasm32-wasi-preview1-threads` and `wasm32-wasip1-threads`
- `napi.wasm` package configuration
- WASI JS binding generation, browser binding generation, and worker templates
- `.wasm` artifact collection, rename, packaging, and pre-publish handling
- the `@napi-rs/wasm-runtime` workspace package
- `@emnapi/*`, `emnapi`, and `@napi-rs/wasm-tools` as direct CLI/example dependencies
- Rust-side WASI build setup, WASM registration exports, and WASM-specific runtime branches

Current behavior:

- Passing a `wasm32-*` target to the CLI target parser is rejected immediately.
- New projects generated from older external templates have stale WASI/browser files filtered out.
- Per-target npm packages are generated only for native `.node` artifacts.
- Native class/module registration uses the descriptor and `linkme` path only.

Lockfile entries for third-party optional WASM packages may still appear when transitive tooling depends on them, for example `@oxc-node/core`, `oxc-parser`, `rolldown`, or `@napi-rs/tar/lzma`. Those entries are not part of this fork's runtime or CLI support surface.

---

## 1. Lifetime Model

Upstream passes raw `napi_value` (`*mut c_void`) everywhere with no lifetime tracking. This fork introduces a two-tier lifetime system:

```rust
// Layer 0: scoped JS value handle
struct Local<'scope, T> { raw: sys::napi_value, .. }

// Scope ties Local lifetime to a NAPI HandleScope
struct Scope<'env, 'scope> {
    env: &'scope mut Env<'env>,
    record: &'scope Rc<EnvRecord>,
    ..
}
```

- `'env` — the `napi_env` is valid (callback is active)
- `'scope` — the `napi_handle_scope` is open (`Local` values are valid)

`Local<'scope, T>` is `Copy` — it's just a pointer with a phantom lifetime. The lifetime prevents escaping a `Local` past its scope.

`EnvRecord` replaces the old scattered thread-local state (`cleanup_env`, `sendable_resolver`, etc.) with a single per-env record managing constructors, instance data, and deferred ref cleanup.

## 2. Value Conversion: `IntoJs` / `FromJs`

Upstream:
```rust
// No lifetime — raw pointer in, raw pointer out
trait ToNapiValue {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value>;
}
trait FromNapiValue {
    unsafe fn from_napi_value(env: sys::napi_env, val: sys::napi_value) -> Result<Self>;
}
```

This fork:
```rust
trait IntoJs<'scope>: Sized {
    type Output;
    fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>>;
}

trait FromJs<'env, 'scope>: Sized {
    fn from_js(scope: &mut Scope<'env, 'scope>, value: Local<'scope, Unknown<'scope>>) -> Result<Self>;
}
```

Key differences:
- **Scoped**: conversion takes `&mut Scope`, returned `Local` is bound to `'scope`
- **Typed output**: `IntoJs::Output` carries the JS type tag (`Number`, `Object`, etc.)
- **Two lifetimes on `FromJs`**: `'env` for env-lifetime-dependent results, `'scope` for the input value
- `ToNapiValue` / `FromNapiValue` are completely removed

## 3. Reference & Ownership Layers

Upstream had 6+ ad-hoc `napi_ref` wrappers (`Reference`, `WeakReference`, `ObjectRef`, `FunctionRef`, etc.) with inconsistent drop behavior — some leaked, some only warned.

This fork uses a **layered model** organized along two dimensions (scoped vs. owned):

```
              scoped ('scope)            owned (Drop)
Layer 0       Local<'scope, T>           Ref<K>  /  WeakRef<K>
Layer 1       ClassLocal<'scope, T>      ClassRef<T>
Layer 2       ClassBorrow<'a, T>         ClassBorrow<'a, T>
              ClassBorrowMut<'a, T>      ClassBorrowMut<'a, T>
```

### Layer 0: `Ref<K>` / `WeakRef<K>`

Generic over `K: JsRefKind` — a marker trait selecting the access pattern:

| Kind         | Type            | Access   |
|--------------|-----------------|----------|
| `Class<T>`   | class instance  | `ClassAccess` (ptr + offset) |
| `Obj`        | plain object    | `()`     |
| `Func<A, R>` | function        | `()`     |
| `Sym`        | symbol          | `()`     |
| `Ext<T>`     | external        | `()`     |
| `Unk`        | unknown         | `()`     |

All refs share `RefState` (raw `napi_ref` + `Weak<EnvRecord>`). Drop is uniform: deferred cleanup via `EnvRecord`. No more "warn then leak".

### Layer 1: `ClassLocal` / `ClassRef`

`ClassLocal<'env, 'scope, T>` — scoped reference to a class instance, holds `Local<Object>` + resolved `ClassStorageRef`. Can `borrow()` / `borrow_mut()`.

`ClassRef<T>` — owned reference with **cached storage pointer** (`NonNull<ClassStorageHeader>`). Can `borrow()` / `borrow_mut()` **without a scope** — this is the main ergonomic win. Useful as a struct field for cross-callback references.

```rust
struct JsRemote {
    repo: ClassRef<JsRepo>,
}

impl JsRemote {
    fn name(&self) -> Result<String> {
        let repo = self.repo.borrow()?;  // no scope needed
        Ok(repo.inner.name())
    }
}
```

### Layer 2: `ClassBorrow` / `ClassBorrowMut`

Guard types backed by `RefCell` — same type regardless of whether the source is scoped or owned. `Deref<Target = T>` / `DerefMut<Target = T>`.

`as_super()` / `as_super_mut()` return `&Parent` / `&mut Parent` directly — the old `SuperRef` / `SuperRefMut` wrapper types are eliminated.

### Type rename table

| Upstream                      | This fork                   |
|-------------------------------|-----------------------------|
| `Reference<T>`                | `Ref<Class<T>>`             |
| `WeakReference<T>`            | `WeakRef<Class<T>>`         |
| `ClassRef<'scope, T>`         | `ClassBorrow<'a, T>`        |
| `ClassRefMut<'scope, T>`      | `ClassBorrowMut<'a, T>`     |
| `SuperRef<'a, P>`             | removed — `as_super() -> &P` |
| `SuperRefMut<'a, P>`          | removed — `as_super_mut() -> &mut P` |
| `ObjectRef`                   | `Ref<Obj>`                  |
| `FunctionRef<A, R>`           | `Ref<Func<A, R>>`           |
| `SymbolRef`                   | `Ref<Sym>`                  |
| `ExternalRef<T>`              | `Ref<Ext<T>>`               |

## 4. Class Inheritance

Upstream has no native class inheritance support. This fork implements it through:

### `NapiClass` / `NapiSubclass` / `NapiReceiver`

```rust
unsafe trait NapiReceiver: Sized + 'static {
    type Access: Copy + Eq + 'static;
    type Borrow<'a>: Deref<Target = Self>;
    type BorrowMut<'a>: DerefMut<Target = Self>;

    fn validate_object(...) -> Result<(Self::Access, ClassStorageRef)>;
    unsafe fn ref_from_validated_object(storage, access) -> Result<Self::Borrow<'_>>;
    unsafe fn mut_from_validated_object(storage, access) -> Result<Self::BorrowMut<'_>>;
}

unsafe trait NapiClass: NapiReceiver<Access = ClassAccess> + 'static {
    type Parent: NativeParent;
    const CLASS: &'static ClassDef<Self>;
}

unsafe trait NapiSubclass: NapiClass {}
```

### `ClassChain` — single-allocation storage

```rust
unsafe trait ClassChain: NapiClass {
    type Layout;
    const LAYOUT: &'static ClassLayout;

    unsafe fn write_init(init: ClassInitializer<Self>, dst: NonNull<Self::Layout>);
    unsafe fn drop_segments(data: NonNull<Self::Layout>);
    unsafe fn drop_initialized(data: NonNull<u8>);
}
```

For a chain `Child extends Parent extends GrandParent`, `ClassChain::Layout` is a flat struct containing all segments. One allocation, one `napi_wrap` call. `ClassStorageHeader` carries ABI magic + version for runtime validation.

### `ClassInitializer<T>`

Recursive initializer: `ClassInitializer { value: T, parent: ClassInitializer<T::Parent> }`, bottoming out at `()` for root classes. Constructed via `#[napi(constructor)]` codegen.

### `ClassAccess`

Pointer + offset pair resolved from `ClassLayout`. Enables `ClassRef::cast::<U>()` — zero-cost type-level downcast/upcast within the inheritance chain.

## 5. Callback Entry

### `CallbackFrame` / `FrameScope` / `FrameObject`

Replaces `CallContext`. Structured callback entry with:

- `CallbackFrame` — holds `FrameScope` + `CallbackValues` (this + args + data)
- `FrameScope` — `Scope` wrapper with frame-local state
- `FrameObject` — arg token consumed exactly once (prevents double-coerce)

### `ConstructorReceiver<T>`

Constructor-specific entry point. Captures `this` raw value + `EnvRecord` weak ref + `ClassInfo` for storage initialization.

## 6. New Features

### `Nullable<T>`

```rust
enum Nullable<T> { Undefined, Null, Value(T) }
```

Distinguishes JS `null` vs `undefined` vs value. `Option<T>` collapses null and undefined into `None`. `FromJs` / `IntoJs` implemented — round-trips correctly.

### `#[napi(env)]` / `#[napi(this)]` / `#[napi(scope)]`

Explicit parameter-level attributes replacing upstream's type-name `strcmp` dispatch. Parameters annotated with these are injected by codegen, not extracted from JS arguments.

```rust
#[napi]
fn my_method(
    #[napi(this)] this: ClassRef<MyClass>,
    #[napi(scope)] scope: &mut Scope<'_, '_>,
    #[napi(env)] env: &Env,
    arg: String,  // from JS
) -> Result<()> { .. }
```

### `#[napi(rest)]`

Variadic parameter capture. Must be last positional arg, type must be `Vec<T>` or `JsArgSlice`.

```rust
#[napi]
fn log(level: String, #[napi(rest)] args: Vec<String>) -> Result<()> { .. }
```

### `#[napi(post_init)]`

Post-construction hook called after the constructor and all parent constructors. Codegen walks the inheritance chain and calls `__napi_post_init` for each class.

### `PromiseFuture<T>`

Wraps a JS `Promise` as a Rust `Future` via channel. Enables `await`-ing JS promises from Rust async code:

```rust
let result: String = promise_future.await?;
```

### `blocking_work`

Replaces `async_work` + `Task` trait. Structured as `execute` (thread pool) + `complete` (event loop) callbacks with `BlockingWorkPromise` handle for cancellation.

### `Ref<K>::with_scope` — external scope recovery

Enables calling JS from outside NAPI callback context (e.g., libuv timers, winit event handlers) by recovering a `Scope` from a `Ref`'s stored `Weak<EnvRecord>`:

```rust
// In a uv_timer / winit event handler — no Scope available
self.handler.with_scope(|scope| {
    let func = scope.borrow_function(&self.handler)?;
    scope.call(&func, FnArgs::from((action, key_data)))
})?;
```

Three-layer entry:

| API | Role |
|-----|------|
| `EnvRecord::current()` | Recover `(napi_env, Rc<EnvRecord>)` from TLS |
| `EnvRecord::enter_external_scope()` | `unsafe` — handle scope + callback scope (napi3) + enter_scope |
| `Ref<K>::with_scope()` | Safe wrapper — current() → record match → enter_external_scope |

`enter_external_scope` opens `napi_open_handle_scope` for V8 handle management, plus (with napi3) `napi_async_init` + `napi_open_callback_scope` for async hooks and microtask checkpoint. Without napi3 it degrades to handle scope only.

Safety is enforced by existing type-system invariants: `Ref: !Send + !Sync` (same thread), `Weak` upgrade (env liveness), HRTB on return type (no scope-bound values escape).

### Enum Class

Data-carrying enums (`enum Foo { Bar { x: i32 }, Baz(String) }`) now participate in the class system when annotated with `#[napi]`. The enum value is stored directly in `ClassStorage` as an opaque class — no fields are auto-exposed. The user controls the JS API surface entirely through `#[napi] impl`:

```rust
#[napi]
pub enum Shape {
  Circle { radius: f64 },
  Rectangle { width: f64, height: f64 },
}

#[napi]
impl Shape {
  #[napi(factory)]
  pub fn circle(radius: f64) -> Shape { Shape::Circle { radius } }

  #[napi(getter)]
  pub fn kind(&self) -> &str {
    match self { Shape::Circle { .. } => "Circle", Shape::Rectangle { .. } => "Rectangle" }
  }

  pub fn area(&self) -> f64 { /* match self ... */ }
}
```

Generated TypeScript:

```typescript
export interface Shape {
  get kind(): string
  area(): number
}
export declare const Shape: {
  circle(radius: number): Shape
  rectangle(width: number, height: number): Shape
  [Symbol.hasInstance](value: unknown): boolean
}
```

The previous behavior (discriminated union of plain objects) is preserved under `#[napi(object)]`:

```rust
#[napi(object, discriminant = "type")]
pub enum StructuredKind { ... }
```

This is a **breaking change**: bare `#[napi]` on data-carrying enums now produces a class instead of a discriminated union. Existing code using discriminated unions must add `object` to the attribute.

## 7. Removed Modules

| Module | Replacement |
|--------|-------------|
| `async_work` | `blocking_work` |
| `task` (Task trait) | `blocking_work` closures |
| `tokio_runtime` (embedded) | libuv `async` driver in `napi`; optional `napi-runtime-tokio` adapter |
| `call_context` | `CallbackFrame` / `FrameScope` |
| `cleanup_env` | `EnvRecord` lifecycle |
| `sendable_resolver` | removed |
| `async_cleanup_hook` | `EnvRecord` drop |
| `compat_macro` | removed |
| `js_values/*` (old pre-bindgen) | `bindgen_runtime` types with lifetimes |
| `promise_raw` | `PromiseFuture` + `blocking_work` |

## 8. Scope Method Migration

Methods previously on `Scope` are now on the types themselves:

| Removed                            | Replacement                      |
|------------------------------------|----------------------------------|
| `scope.bind_reference(&ref)`       | `ref.as_class_local(scope)`      |
| `scope.borrow_class(&local)`       | `local.borrow()`                 |
| `scope.borrow_class_mut(&local)`   | `local.borrow_mut()`             |
| `scope.clone_reference(&ref)`      | `ref.clone(scope)`               |
| `scope.downgrade_reference(&ref)`  | `ref.downgrade(scope)`           |
| `scope.upgrade_reference(&weak)`   | `weak.upgrade(scope)`            |
| `scope.close_reference(ref)`       | `ref.close(scope)`               |
| `scope.create_reference(&local)`   | `local.to_ref(scope)`            |
| `scope.create_weak_reference(&local)` | `local.to_weak_ref(scope)`    |

## 9. Async Runtime (libuv + LocalExecutor)

Upstream (and the earlier fork) embedded a **Tokio `Runtime` in a background thread** when `tokio_rt` / `async` was enabled. Module init started it; `Env::spawn_future` scheduled work on that pool and resolved promises back on the JS thread via `JsDeferred`.

This fork replaces that model with a **per-`napi_env` libuv-driven driver**:

```
JS thread                         other threads
   │                                    │
   │  LocalExecutor polls Runnable      │  channel.push(closure)
   │  from async_task                   ├──────────────────────────►
   │                                    │
   │  uv_async_send → drain queue       │
   │  → resolve/reject deferred           │
```

`EnvRecord` owns an `AsyncDriver` (created on first use, torn down with the env). `AsyncChannel` wraps `uv_async_t` for cross-thread dispatch; registered handles are closed via `uv_close` on drop.

### Feature flags

| Before | After |
|--------|-------|
| `async = ["tokio_rt"]` | `async = ["async-task", "napi4"]` |
| `web_stream` depended on `tokio_rt` | `web_stream = ["futures-core", "napi5", "async"]` |

`tokio` remains an **optional dependency** for `tokio_*` feature flags (`tokio_fs`, `tokio_net`, …). It is no longer required to run `#[napi] async fn` exports.

### Breaking API renames

| Removed / old | Replacement |
|---------------|-------------|
| `Env::spawn_future` | `Env::spawn_promise` |
| `Env::spawn_future_with_callback` | `Env::spawn_promise_with` |
| `Env::spawn_future_with_callback_and_finalize` | `spawn_promise_with` (completion always runs; see below) |
| `Env::create_deferred` | `Env::deferred` |
| `#[napi(async_runtime)]` | removed — use `napi-runtime-tokio` if Tokio context is needed |
| `create_custom_tokio_runtime` / `start_async_runtime` / `shutdown_async_runtime` | removed from `napi` |
| `within_runtime_if_available` | removed |
| `napi::tokio::*` re-exports | use `tokio` directly, or `napi-runtime-tokio` for poll integration |

### `spawn_promise_with` completion contract

Codegen and manual callers use a completion callback that receives **`Result<T>`**, not `T`:

```rust
env.spawn_promise_with(fut, |scope, result| {
    let value = result?;  // async Err → promise reject
    Ok(transform(scope, value)?)
})?;
```

The completion runs on **success, async `Err`, and panic** paths so `AsyncArgRefs::finalize` always executes. `spawn_promise` is `spawn_promise_with(fut, |_, r| r)`.

If `spawn_future` fails after the deferred handle is created, the promise is **rejected** instead of staying pending.

### `DeferredCompletion` / `JsDeferred`

Promise settle/reject is centralized in `DeferredCompletion` (stack stitching for async errors, `AsyncKeepAlive` for env lifetime). Pending state is held in `Option<PendingState>` and taken before `napi_resolve_deferred` / `napi_reject_deferred`, avoiding false "dropped unsettled" warnings during normal settle.

`JsDeferred::resolve` / `reject` still dispatch through `AsyncChannel` for cross-thread use.

### `napi-runtime-tokio` (new workspace crate)

Tokio is **opt-in** at the addon boundary:

```rust
#[napi(module_exports)]
pub fn exports(#[napi(env)] env: Env, export: Object) -> Result<()> {
    napi_runtime_tokio::install_factory(&env, || {
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
    })?;
    // ...
}
```

| API | Purpose |
|-----|---------|
| `install` | Own a `Runtime` for the env's poll context |
| `install_factory` | Lazy-create `Runtime` on first async poll |
| `install_handle` / `install_current` | Use an existing Tokio handle |

The driver calls the installed context's `enter()` around each `Runnable` poll so `tokio::fs`, timers, etc. work inside `#[napi] async fn` without embedding Tokio inside `napi` itself.

### Other async-related changes

- `#[napi] async fn` futures no longer require `Send` — they are polled on the main thread.
- `web_stream` pull paths use `futures::lock::Mutex` and `spawn_promise_with` instead of `tokio::sync::Mutex` + `spawn_future_with_callback`.
- Module registration no longer calls `start_async_runtime` / `shutdown_async_runtime` on load/unload.
