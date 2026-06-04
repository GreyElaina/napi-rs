use proc_macro2::Ident;
use syn::{GenericArgument, PathArguments, Type};

pub(crate) fn forbidden_class_field_type(ty: &Type) -> Option<(Ident, String)> {
  const FORBIDDEN: &[&str] = &[
    "AbortSignal",
    "ArrayBuffer",
    "Buffer",
    "BufferSlice",
    "Env",
    "FrameScope",
    "FunctionCallContext",
    "ClassLocal",
    "ClassBorrow",
    "ClassBorrowMut",
    "ClassStorageRef",
    "CleanupEnvHook",
    "Date",
    "EscapableHandleScope",
    "HandleScope",
    "Object",
    "Promise",
    "PromiseFuture",
    "JsArrayBuffer",
    "JsArrayBufferValue",
    "JsBigInt",
    "JsBoolean",
    "JsBuffer",
    "JsBufferValue",
    "JsDataView",
    "JsDataViewValue",
    "JsDate",
    "JsExternal",
    "JsFunction",
    "JsGlobal",
    "JsNull",
    "JsNumber",
    "JsObject",
    "JsString",
    "JsStringLatin1",
    "JsStringUtf8",
    "JsStringUtf16",
    "JsSymbol",
    "JsTimeout",
    "JsTypedArray",
    "JsTypedArrayValue",
    "JsUndefined",
    "JsUnknown",
    "JSON",
    "JsDeferred",
    "Array",
    "BigInt64Array",
    "BigInt64ArraySlice",
    "BigUint64Array",
    "BigUint64ArraySlice",
    "Float32Array",
    "Float32ArraySlice",
    "Float64Array",
    "Float64ArraySlice",
    "Int16Array",
    "Int16ArraySlice",
    "Int32Array",
    "Int32ArraySlice",
    "Int8Array",
    "Int8ArraySlice",
    "IteratorValue",
    "ReadableStream",
    "This",
    "TypedArray",
    "Uint16Array",
    "Uint16ArraySlice",
    "Uint32Array",
    "Uint32ArraySlice",
    "Uint8Array",
    "Uint8ArraySlice",
    "Uint8ClampedArray",
    "Uint8ClampedSlice",
    "WriteableStream",
    "Unknown",
    "Function",
    "napi_env",
    "napi_value",
    "napi_ref",
  ];

  match ty {
    Type::Path(syn::TypePath { path, .. }) => {
      let segment = path.segments.last()?;
      let name = segment.ident.to_string();
      if FORBIDDEN.contains(&name.as_str()) || is_raw_napi_type_name(&name) {
        return Some((segment.ident.clone(), name));
      }
      if let PathArguments::AngleBracketed(args) = &segment.arguments {
        for arg in &args.args {
          if let GenericArgument::Type(ty) = arg {
            if let Some(forbidden) = forbidden_class_field_type(ty) {
              return Some(forbidden);
            }
          }
        }
      }
      None
    }
    Type::Reference(reference) => forbidden_class_field_type(&reference.elem),
    Type::Ptr(pointer) => forbidden_class_field_type(&pointer.elem),
    Type::Array(array) => forbidden_class_field_type(&array.elem),
    Type::Slice(slice) => forbidden_class_field_type(&slice.elem),
    Type::Group(group) => forbidden_class_field_type(&group.elem),
    Type::Paren(paren) => forbidden_class_field_type(&paren.elem),
    Type::Tuple(tuple) => tuple.elems.iter().find_map(forbidden_class_field_type),
    _ => None,
  }
}

fn is_raw_napi_type_name(name: &str) -> bool {
  name.starts_with("napi_")
}

pub(crate) fn forbidden_js_visible_type(ty: &Type) -> Option<(Ident, &'static str)> {
  match ty {
    Type::Path(path) => {
      for segment in &path.path.segments {
        if segment.ident == "WeakRef" || segment.ident == "WeakReference" {
          return Some((
            segment.ident.clone(),
            "WeakRef<T> cannot be used in JavaScript-visible signatures",
          ));
        }
        if is_raw_napi_type_name(&segment.ident.to_string()) {
          return Some((
            segment.ident.clone(),
            "raw Node-API handles cannot be used in JavaScript-visible signatures",
          ));
        }
        if let PathArguments::AngleBracketed(args) = &segment.arguments {
          for arg in &args.args {
            if let GenericArgument::Type(ty) = arg {
              if let Some(ident) = forbidden_js_visible_type(ty) {
                return Some(ident);
              }
            }
          }
        }
      }
      None
    }
    Type::Reference(reference) => forbidden_js_visible_type(&reference.elem),
    Type::Ptr(pointer) => forbidden_js_visible_type(&pointer.elem),
    Type::Array(array) => forbidden_js_visible_type(&array.elem),
    Type::Slice(slice) => forbidden_js_visible_type(&slice.elem),
    Type::Group(group) => forbidden_js_visible_type(&group.elem),
    Type::Paren(paren) => forbidden_js_visible_type(&paren.elem),
    Type::Tuple(tuple) => tuple.elems.iter().find_map(forbidden_js_visible_type),
    _ => None,
  }
}
