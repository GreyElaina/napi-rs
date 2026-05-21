use napi::{bindgen_prelude::*, JsSymbol, SymbolRef};

#[napi]
pub fn set_symbol_in_obj<'scope>(#[napi(env)] env: &'scope Env, symbol: JsSymbol) -> Result<Object<'scope>> {
  let mut obj = Object::new(env)?;
  obj.set_property(symbol, env.create_string("a symbol")?)?;
  Ok(obj)
}

#[napi]
pub fn create_symbol() -> Symbol {
  Symbol::new("a symbol".to_owned())
}

#[napi]
pub fn create_symbol_for(desc: String) -> Symbol {
  Symbol::for_desc(desc)
}

#[napi]
pub fn create_symbol_ref(#[napi(env)] env: &mut Env, desc: String) -> Result<SymbolRef> {
  env.with_scope(|scope| scope.create_ref(Symbol::for_desc(desc)))
}
