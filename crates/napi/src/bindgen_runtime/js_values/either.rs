use super::{FromJs, IntoJs, Local, Scope, TypeName};
use crate::{
  bindgen_runtime::{Null, Undefined, Unknown},
  JsValue, Status, ValueType,
};

impl<T> From<Option<T>> for Either<T, Undefined> {
  fn from(value: Option<T>) -> Self {
    match value {
      Some(v) => Either::A(v),
      None => Either::B(()),
    }
  }
}

impl<T> From<Either<T, Null>> for Option<T> {
  fn from(value: Either<T, Null>) -> Option<T> {
    match value {
      Either::A(v) => Some(v),
      Either::B(_) => None,
    }
  }
}

macro_rules! either_n {
  ( $either_name:ident, $( $parameter:ident ),+ $( , )* ) => {
    #[derive(Debug, Clone, Copy)]
    pub enum $either_name< $( $parameter ),+ > {
      $( $parameter ( $parameter ) ),+
    }

    impl< $( $parameter ),+ > TypeName for $either_name < $( $parameter ),+ >
      where $( $parameter: TypeName ),+
    {
      fn type_name() -> &'static str {
        stringify!( $either_name )
      }

      fn value_type() -> ValueType {
        ValueType::Unknown
      }

      fn ts_type() -> String {
        let types = vec![$($parameter::ts_type()),+];
        types.join(" | ")
      }
    }

    impl<'env, 'scope, $( $parameter ),+ > FromJs<'env, 'scope> for $either_name < $( $parameter ),+ >
      where $( $parameter: TypeName + FromJs<'env, 'scope> ),+
    {
      fn from_js(
        scope: &mut Scope<'env, 'scope>,
        value: Local<'scope, Unknown<'scope>>,
      ) -> crate::Result<Self> {
        $(
          if let Ok(value) = $parameter::from_js(scope, value) {
            return Ok(Self:: $parameter(value));
          }
        )+

        Err(crate::Error::new(
          Status::InvalidArg,
          format!(
            concat!("Value is none of these types ", $( "`{", stringify!( $parameter ), "}`, " ),+ ),
            $( $parameter = $parameter::type_name(), )+
          ),
        ))
      }
    }

    impl<'scope, $( $parameter ),+ > IntoJs<'scope> for $either_name < $( $parameter ),+ >
      where $( $parameter: IntoJs<'scope> + 'scope ),+
    {
      type Output = Unknown<'scope>;

      fn into_js(
        self,
        scope: &mut Scope<'_, 'scope>,
      ) -> crate::Result<Local<'scope, Self::Output>> {
        match self {
          $( Self:: $parameter (v) => v.into_js(scope).map(|local| unsafe { Local::from_raw(local.raw()) }) ),+
        }
      }
    }

    impl<Data, $( $parameter: AsRef<Data> ),+ > AsRef<Data> for $either_name < $( $parameter ),+ >
      where Data: ?Sized,
    {
      fn as_ref(&self) -> &Data {
        match &self {
          $( Self:: $parameter (v) => v.as_ref() ),+
        }
      }
    }

    impl<'env, $( $parameter ),+ > $either_name < $( $parameter ),+ >
      where $( $parameter: JsValue<'env> ),+
    {
      pub fn as_unknown(&self) -> Unknown<'env> {
        match &self {
          $( Self:: $parameter (v) => v.to_unknown() ),+
        }
      }
    }

    #[cfg(feature = "serde-json")]
    impl< $( $parameter: serde::Serialize ),+ > serde::Serialize for $either_name< $( $parameter ),+ > {
      fn serialize<Ser>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error>
      where
        Ser: serde::Serializer
      {
        match &self {
          $( Self:: $parameter (v) => serializer.serialize_some(v) ),+
        }
      }
    }
  };
}

either_n!(Either, A, B);
either_n!(Either3, A, B, C);
either_n!(Either4, A, B, C, D);
either_n!(Either5, A, B, C, D, E);
either_n!(Either6, A, B, C, D, E, F);
either_n!(Either7, A, B, C, D, E, F, G);
either_n!(Either8, A, B, C, D, E, F, G, H);
either_n!(Either9, A, B, C, D, E, F, G, H, I);
either_n!(Either10, A, B, C, D, E, F, G, H, I, J);
either_n!(Either11, A, B, C, D, E, F, G, H, I, J, K);
either_n!(Either12, A, B, C, D, E, F, G, H, I, J, K, L);
either_n!(Either13, A, B, C, D, E, F, G, H, I, J, K, L, M);
either_n!(Either14, A, B, C, D, E, F, G, H, I, J, K, L, M, N);
either_n!(Either15, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
either_n!(Either16, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
either_n!(Either17, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q);
either_n!(Either18, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R);
either_n!(Either19, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S);
either_n!(Either20, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T);
either_n!(Either21, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U);
either_n!(Either22, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V);
either_n!(Either23, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W);
either_n!(Either24, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X);
either_n!(Either25, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y);
either_n!(Either26, A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);
