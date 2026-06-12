use std::marker::PhantomData;
use std::sync::Arc;

use crate::{
  bindgen_runtime::{
    js_values::value_ref::{
      create_reference, delete_reference, ensure_deferred_match_env, ensure_same_deferred,
      reference_value, RefState,
    },
    Env, FromJs, IntoJs, JsRefTarget, Local, Ref, Scope, Sym, Symbol, TypeName, Unknown,
    ValidateNapiValue,
  },
  JsValue, Result, Value, ValueType,
};

#[derive(Clone, Copy)]
/// represent `Symbol` value in JavaScript
pub struct JsSymbol<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl TypeName for JsSymbol<'_> {
  fn type_name() -> &'static str {
    "symbol"
  }

  fn value_type() -> ValueType {
    ValueType::Symbol
  }
}

impl<'env> JsValue<'env> for JsSymbol<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for JsSymbol<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(JsSymbol(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Symbol,
      },
      PhantomData,
    ))
  }
}

impl ValidateNapiValue for JsSymbol<'_> {}

pub type SymbolRef = Ref<Sym>;

impl<'scope> JsRefTarget<'scope, Ref<Sym>> for &JsSymbol<'_> {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Sym>> {
    scope.ensure_value_env(self.0.env, "Symbol")?;
    let raw = create_reference(scope.env().raw(), self.0.value, 1)?;
    Ok(Ref::new(
      RefState::new(raw, Arc::clone(scope.deferred_queue())),
      (),
    ))
  }
}

impl<'scope> JsRefTarget<'scope, Ref<Sym>> for Symbol {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Sym>> {
    let value = self.into_js(scope)?;
    let raw = create_reference(scope.env().raw(), value.raw(), 1)?;
    Ok(Ref::new(
      RefState::new(raw, Arc::clone(scope.deferred_queue())),
      (),
    ))
  }
}

impl Ref<Sym> {
  pub fn to_local<'env>(&self, env: &'env Env) -> Result<JsSymbol<'env>> {
    ensure_deferred_match_env(&self.state, env)?;
    let result = reference_value(env.0, self.state.raw_ref()?)?;
    Ok(JsSymbol(
      Value {
        env: env.0,
        value: result,
        value_type: ValueType::Symbol,
      },
      PhantomData,
    ))
  }

  pub fn close(self, env: &Env) -> Result<()> {
    ensure_deferred_match_env(&self.state, env)?;
    delete_reference(env.0, self.state.take_raw()?)
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Ref<Sym> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = JsSymbol::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope> IntoJs<'scope> for &Ref<Sym> {
  type Output = JsSymbol<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ensure_same_deferred(&self.state, scope)?;
    let result = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope> IntoJs<'scope> for Ref<Sym> {
  type Output = JsSymbol<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ensure_same_deferred(&self.state, scope)?;
    let raw_ref = self.state.raw_ref()?;
    let result = reference_value(scope.env().raw(), raw_ref)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}
