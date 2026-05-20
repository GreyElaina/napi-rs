use serde::de::IntoDeserializer;
use serde::de::Visitor;
use serde::de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, Unexpected, VariantAccess};

#[cfg(feature = "napi6")]
use crate::bindgen_runtime::BigInt;
use crate::{
  bindgen_runtime::{
    ArrayBuffer, BufferSlice, FromJs, JsObjectValue, Local, Object, Scope, Uint8ArraySlice, Unknown,
  },
  type_of, Error, JsValue, Result, Status, ValueType,
};

pub struct De<'env, 'scope, 'de> {
  scope: &'de mut Scope<'env, 'scope>,
  value: Local<'scope, Unknown<'scope>>,
}

impl<'env, 'scope, 'de> De<'env, 'scope, 'de> {
  pub fn new(scope: &'de mut Scope<'env, 'scope>, value: Local<'scope, Unknown<'scope>>) -> Self {
    Self { scope, value }
  }

  fn value_type(&self) -> Result<ValueType> {
    type_of!(self.scope.env().raw(), self.value.raw())
  }

  fn object(&mut self) -> Result<Object<'scope>> {
    Object::from_js(self.scope, self.value)
  }
}

#[doc(hidden)]
impl<'serde, 'env, 'scope> serde::de::Deserializer<'serde> for &mut De<'env, 'scope, '_> {
  type Error = Error;

  fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    let js_value_type = self.value_type()?;
    match js_value_type {
      ValueType::Null | ValueType::Undefined => visitor.visit_unit(),
      ValueType::Boolean => visitor.visit_bool(bool::from_js(self.scope, self.value)?),
      ValueType::Number => {
        let js_number = f64::from_js(self.scope, self.value)?;
        if (js_number.trunc() - js_number).abs() < f64::EPSILON {
          visitor.visit_i64(js_number as i64)
        } else {
          visitor.visit_f64(js_number)
        }
      }
      ValueType::String => {
        let value = String::from_js(self.scope, self.value)?;
        visitor.visit_str(value.as_str())
      }
      ValueType::Object => {
        let js_object = self.object()?;
        if js_object.is_array()? {
          let mut deserializer = JsArrayAccess::new(
            self.scope,
            js_object,
            js_object.get_array_length_unchecked()?,
          );
          visitor.visit_seq(&mut deserializer)
        } else if js_object.is_typedarray()? {
          let value = Uint8ArraySlice::from_js(self.scope, self.value)?;
          visitor.visit_bytes(value.as_ref())
        } else if js_object.is_buffer()? {
          let value = BufferSlice::from_js(self.scope, self.value)?;
          visitor.visit_bytes(value.as_ref())
        } else if js_object.is_arraybuffer()? {
          let array_buf = ArrayBuffer::from_js(self.scope, self.value)?;
          if array_buf.data.is_empty() {
            return visitor.visit_bytes(&[]);
          }
          visitor.visit_bytes(array_buf.data)
        } else {
          let mut deserializer = JsObjectAccess::new(self.scope, js_object)?;
          visitor.visit_map(&mut deserializer)
        }
      }
      #[cfg(feature = "napi6")]
      ValueType::BigInt => {
        let js_bigint = BigInt::from_js(self.scope, self.value)?;

        let BigInt { sign_bit, words } = &js_bigint;
        let word_sized = words.len() < 2;

        match (sign_bit, word_sized) {
          (true, true) => visitor.visit_i64(js_bigint.get_i64().0),
          (true, false) => visitor.visit_i128(js_bigint.get_i128().0),
          (false, true) => visitor.visit_u64(js_bigint.get_u64().1),
          (false, false) => visitor.visit_u128(js_bigint.get_u128().1),
        }
      }
      ValueType::External | ValueType::Function | ValueType::Symbol => Err(Error::new(
        Status::InvalidArg,
        format!("typeof {js_value_type:?} value could not be deserialized"),
      )),
      ValueType::Unknown => unreachable!(),
    }
  }

  fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    match self.value_type()? {
      ValueType::Object => {
        let js_object = self.object()?;
        if js_object.is_buffer()? {
          let value = BufferSlice::from_js(self.scope, self.value)?;
          return visitor.visit_bytes(value.as_ref());
        } else if js_object.is_typedarray()? {
          let value = Uint8ArraySlice::from_js(self.scope, self.value)?;
          return visitor.visit_bytes(value.as_ref());
        } else if js_object.is_arraybuffer()? {
          let array_buf = ArrayBuffer::from_js(self.scope, self.value)?;
          if array_buf.data.is_empty() {
            return visitor.visit_bytes(&[]);
          }
          return visitor.visit_bytes(array_buf.data);
        }
        self.deserialize_any(visitor)
      }
      _ => self.deserialize_any(visitor),
    }
  }

  fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    match self.value_type()? {
      ValueType::Object => {
        let js_object = self.object()?;
        if js_object.is_buffer()? {
          let value = BufferSlice::from_js(self.scope, self.value)?;
          return visitor.visit_byte_buf(value.as_ref().to_vec());
        } else if js_object.is_typedarray()? {
          let value = Uint8ArraySlice::from_js(self.scope, self.value)?;
          return visitor.visit_byte_buf(value.as_ref().to_vec());
        } else if js_object.is_arraybuffer()? {
          let array_buf = ArrayBuffer::from_js(self.scope, self.value)?;
          if array_buf.data.is_empty() {
            return visitor.visit_byte_buf(Vec::new());
          }
          return visitor.visit_byte_buf(array_buf.data.to_vec());
        }
        self.deserialize_any(visitor)
      }
      _ => self.deserialize_any(visitor),
    }
  }

  fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    match self.value_type()? {
      ValueType::Undefined | ValueType::Null => visitor.visit_none(),
      _ => visitor.visit_some(self),
    }
  }

  fn deserialize_enum<V>(
    self,
    _name: &'static str,
    _variants: &'static [&'static str],
    visitor: V,
  ) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    let js_value_type = self.value_type()?;
    match js_value_type {
      ValueType::String => {
        let variant = String::from_js(self.scope, self.value)?;
        visitor.visit_enum(JsEnumAccess::new(self.scope, variant, None))
      }
      ValueType::Object => {
        let js_object = self.object()?;
        let keys = self.scope.keys(&js_object)?;
        if keys.len() != 1 {
          Err(Error::new(
            Status::InvalidArg,
            format!(
              "object key length: {}, can not deserialize to Enum",
              keys.len()
            ),
          ))
        } else {
          let key = keys.into_iter().next().ok_or_else(|| {
            Error::new(
              Status::InvalidArg,
              "object key length changed while deserializing Enum",
            )
          })?;
          let value: Unknown = self.scope.get_named_property(&js_object, &key)?;
          visitor.visit_enum(JsEnumAccess::new(self.scope, key, Some(value.into_local())))
        }
      }
      _ => Err(Error::new(
        Status::InvalidArg,
        format!("{js_value_type:?} type could not deserialize to Enum type"),
      )),
    }
  }

  fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    visitor.visit_unit()
  }

  forward_to_deserialize_any! {
     <V: Visitor<'serde>>
      bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
      unit unit_struct seq tuple tuple_struct map struct identifier
      newtype_struct
  }
}

#[doc(hidden)]
pub(crate) struct JsEnumAccess<'env, 'scope, 'access> {
  scope: &'access mut Scope<'env, 'scope>,
  variant: String,
  value: Option<Local<'scope, Unknown<'scope>>>,
}

#[doc(hidden)]
impl<'env, 'scope, 'access> JsEnumAccess<'env, 'scope, 'access> {
  fn new(
    scope: &'access mut Scope<'env, 'scope>,
    variant: String,
    value: Option<Local<'scope, Unknown<'scope>>>,
  ) -> Self {
    Self {
      scope,
      variant,
      value,
    }
  }
}

#[doc(hidden)]
impl<'serde, 'env, 'scope, 'access> EnumAccess<'serde> for JsEnumAccess<'env, 'scope, 'access> {
  type Error = Error;
  type Variant = JsVariantAccess<'env, 'scope, 'access>;

  fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant)>
  where
    V: DeserializeSeed<'serde>,
  {
    let variant = self.variant.into_deserializer();
    let variant_access = JsVariantAccess {
      scope: self.scope,
      value: self.value,
    };
    seed.deserialize(variant).map(|v| (v, variant_access))
  }
}

#[doc(hidden)]
pub(crate) struct JsVariantAccess<'env, 'scope, 'access> {
  scope: &'access mut Scope<'env, 'scope>,
  value: Option<Local<'scope, Unknown<'scope>>>,
}

#[doc(hidden)]
impl<'serde, 'env, 'scope> VariantAccess<'serde> for JsVariantAccess<'env, 'scope, '_> {
  type Error = Error;

  fn unit_variant(self) -> Result<()> {
    match self.value {
      Some(value) => {
        let mut deserializer = De::new(self.scope, value);
        serde::de::Deserialize::deserialize(&mut deserializer)
      }
      None => Ok(()),
    }
  }

  fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value>
  where
    T: DeserializeSeed<'serde>,
  {
    match self.value {
      Some(value) => {
        let mut deserializer = De::new(self.scope, value);
        seed.deserialize(&mut deserializer)
      }
      None => Err(serde::de::Error::invalid_type(
        Unexpected::UnitVariant,
        &"newtype variant",
      )),
    }
  }

  fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    match self.value {
      Some(value) => {
        let js_object = Object::from_js(self.scope, value)?;
        if js_object.is_array()? {
          let mut deserializer = JsArrayAccess::new(
            self.scope,
            js_object,
            js_object.get_array_length_unchecked()?,
          );
          visitor.visit_seq(&mut deserializer)
        } else {
          Err(serde::de::Error::invalid_type(
            Unexpected::Other("JsValue"),
            &"tuple variant",
          ))
        }
      }
      None => Err(serde::de::Error::invalid_type(
        Unexpected::UnitVariant,
        &"tuple variant",
      )),
    }
  }

  fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
  where
    V: Visitor<'serde>,
  {
    match self.value {
      Some(value) => {
        let js_object = Object::from_js(self.scope, value)?;
        let mut deserializer = JsObjectAccess::new(self.scope, js_object)?;
        visitor.visit_map(&mut deserializer)
      }
      None => Err(serde::de::Error::invalid_type(
        Unexpected::UnitVariant,
        &"struct variant",
      )),
    }
  }
}

#[doc(hidden)]
struct JsArrayAccess<'env, 'scope, 'access> {
  scope: &'access mut Scope<'env, 'scope>,
  input: Object<'scope>,
  idx: u32,
  len: u32,
}

#[doc(hidden)]
impl<'env, 'scope, 'access> JsArrayAccess<'env, 'scope, 'access> {
  fn new(scope: &'access mut Scope<'env, 'scope>, input: Object<'scope>, len: u32) -> Self {
    Self {
      scope,
      input,
      idx: 0,
      len,
    }
  }
}

#[doc(hidden)]
impl<'serde, 'env, 'scope> SeqAccess<'serde> for JsArrayAccess<'env, 'scope, '_> {
  type Error = Error;

  fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
  where
    T: DeserializeSeed<'serde>,
  {
    if self.idx >= self.len {
      return Ok(None);
    }
    let value: Unknown = self.scope.get_element(&self.input, self.idx)?;
    self.idx += 1;

    let mut de = De::new(self.scope, value.into_local());
    seed.deserialize(&mut de).map(Some)
  }
}

#[doc(hidden)]
pub(crate) struct JsObjectAccess<'env, 'scope, 'access> {
  scope: &'access mut Scope<'env, 'scope>,
  value: Object<'scope>,
  keys: Vec<String>,
  idx: usize,
}

#[doc(hidden)]
impl<'env, 'scope, 'access> JsObjectAccess<'env, 'scope, 'access> {
  fn new(scope: &'access mut Scope<'env, 'scope>, value: Object<'scope>) -> Result<Self> {
    let keys = scope.keys(&value)?;
    Ok(Self {
      scope,
      value,
      keys,
      idx: 0,
    })
  }
}

#[doc(hidden)]
impl<'serde, 'env, 'scope> MapAccess<'serde> for JsObjectAccess<'env, 'scope, '_> {
  type Error = Error;

  fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
  where
    K: DeserializeSeed<'serde>,
  {
    let Some(prop_name) = self.keys.get(self.idx) else {
      return Ok(None);
    };
    seed
      .deserialize(prop_name.as_str().into_deserializer())
      .map(Some)
  }

  fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
  where
    V: DeserializeSeed<'serde>,
  {
    let prop_name = self.keys.get(self.idx).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        format!("Index:{} out of range: {}", self.keys.len(), self.idx),
      )
    })?;
    let value: Unknown = self.scope.get_named_property(&self.value, prop_name)?;

    self.idx += 1;
    let mut de = De::new(self.scope, value.into_local());
    seed.deserialize(&mut de)
  }
}
