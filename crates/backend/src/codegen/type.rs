use proc_macro2::TokenStream;
use quote::ToTokens;

use crate::{BindgenResult, NapiType, TryToTokens};

#[cfg(feature = "type-def")]
fn has_free_type_params(ty: &syn::Type) -> bool {
  use syn::visit::Visit;
  static KNOWN_IDENTS: &[&str] = &[
    "Option",
    "Result",
    "Vec",
    "String",
    "Box",
    "Rc",
    "Arc",
    "Mutex",
    "HashMap",
    "BTreeMap",
    "HashSet",
    "BTreeSet",
    "IndexMap",
    "IndexSet",
    "Either",
    "Either3",
    "Either4",
    "Either5",
    "Promise",
    "PromiseFuture",
    "Function",
    "FunctionRef",
    "FnArgs",
    "Ref",
    "ClassRef",
    "ClassBorrow",
    "ClassBorrowMut",
    "Class",
    "External",
    "ExternalRef",
    "Buffer",
    "BufferSlice",
    "Null",
    "Undefined",
    "JsUnknown",
    "Unknown",
    "UnknownRef",
    "ClassInitializer",
    "AbortSignal",
    "Scope",
    "Env",
    "Self",
    "bool",
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "f32",
    "f64",
    "usize",
    "isize",
    "str",
    "BigInt",
    "ReadableStream",
    "Nullable",
  ];
  struct Checker(bool);
  impl<'ast> Visit<'ast> for Checker {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
      if path.qself.is_none()
        && path.path.segments.len() == 1
        && path.path.segments[0].arguments.is_empty()
      {
        let name = path.path.segments[0].ident.to_string();
        if !KNOWN_IDENTS.contains(&name.as_str()) {
          self.0 = true;
        }
      }
      syn::visit::visit_type_path(self, path);
    }
  }
  let mut checker = Checker(false);
  checker.visit_type(ty);
  checker.0
}

impl TryToTokens for NapiType {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()> {
    let type_def_register = self.gen_type_def_register();
    (quote! {
      #type_def_register
    })
    .to_tokens(tokens);
    Ok(())
  }
}

impl NapiType {
  #[cfg(feature = "type-def")]
  fn gen_type_def_register(&self) -> TokenStream {
    if self.skip_typescript || cfg!(test) {
      return quote! {};
    }

    let js_name = &self.js_name;
    let def_expr = match &self.ts_type {
      Some(ts) => quote! { String::from(#ts) },
      None => {
        if has_free_type_params(&self.value) {
          return quote! {};
        }
        crate::typegen::tokens::ty_to_ts_type_tokens(&self.value, false, false, None).0
      }
    };

    super::emit_type_def_descriptor(
      "type",
      js_name,
      Some(&self.name.to_string()),
      def_expr,
      self.js_mod.as_ref(),
      &crate::typegen::JSDoc::new(&self.comments),
      None,
      None,
      &self.register_name,
      self.name.span(),
    )
  }

  #[cfg(not(feature = "type-def"))]
  fn gen_type_def_register(&self) -> TokenStream {
    quote! {}
  }
}
