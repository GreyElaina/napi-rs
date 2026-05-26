# Fork Changes

Diverged from upstream napi-rs at [`e9c50bb4`](https://github.com/napi-rs/napi-rs/commit/e9c50bb4) (v3 release).

33 commits, 98 files changed (+13k/−11k lines). The core theme is **lifetime-safe bindings** — encoding JavaScript value scoping rules into Rust's type system so that use-after-free, dangling ref, and env-mismatch bugs become compile errors.

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

## 7. Removed Modules

| Module | Replacement |
|--------|-------------|
| `async_work` | `blocking_work` |
| `task` (Task trait) | `blocking_work` closures |
| `tokio_runtime` | removed (external runtime management) |
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
