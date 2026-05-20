use std::{cell::RefCell, rc::Rc};

use napi::bindgen_prelude::*;

pub struct Repository {
  dir: String,
}

impl Repository {
  fn remote(&self) -> Remote<'_> {
    Remote { inner: self }
  }
}

pub struct Remote<'repo> {
  inner: &'repo Repository,
}

impl Remote<'_> {
  fn name(&self) -> String {
    "origin".to_owned()
  }
}

#[napi]
pub struct JsRepo {
  inner: Repository,
}

#[napi]
impl JsRepo {
  #[napi(constructor)]
  pub fn new(dir: String) -> Self {
    JsRepo {
      inner: Repository { dir },
    }
  }

  #[napi]
  pub fn remote(&self, this: Reference<JsRepo>) -> ClassInitializer<JsRemote> {
    ClassInitializer::from(JsRemote { repo: this })
  }
}

#[napi]
pub struct JsRemote {
  repo: Reference<JsRepo>,
}

#[napi]
impl JsRemote {
  #[napi(constructor)]
  pub fn new(repo: Reference<JsRepo>) -> Self {
    Self { repo }
  }

  #[napi]
  pub fn name(&self, mut env: Env) -> Result<String> {
    env.with_scope(|scope| {
      let repo_ref = scope.bind_reference(&self.repo)?;
      let repo = scope.borrow_class(&repo_ref)?;
      Ok(repo.inner.remote().name())
    })
  }
}

struct OwnedStyleSheet {
  rules: Vec<String>,
}

#[napi]
pub struct CSSRuleList {
  owned: Rc<RefCell<OwnedStyleSheet>>,
  parent: WeakReference<CSSStyleSheet>,
}

#[napi]
impl CSSRuleList {
  #[napi]
  pub fn get_rules(&self) -> Vec<String> {
    self.owned.borrow().rules.to_vec()
  }

  #[napi(getter)]
  pub fn name(&self, mut env: Env) -> Result<Option<String>> {
    env.with_scope(|scope| {
      let Some(stylesheet) = scope.upgrade_reference(&self.parent)? else {
        return Ok(None);
      };
      let stylesheet_ref = scope.bind_reference(&stylesheet)?;
      let stylesheet = scope.borrow_class(&stylesheet_ref)?;
      Ok(Some(stylesheet.name.clone()))
    })
  }

  #[napi(getter)]
  pub fn parent_style_sheet(&self, mut env: Env) -> Result<Reference<CSSStyleSheet>> {
    env.with_scope(|scope| {
      scope.upgrade_reference(&self.parent)?.ok_or_else(|| {
        Error::new(
          Status::GenericFailure,
          "Parent stylesheet has been dropped".to_owned(),
        )
      })
    })
  }
}

#[napi]
pub struct CSSStyleSheet {
  name: String,
  inner: Rc<RefCell<OwnedStyleSheet>>,
  rules: Option<Reference<CSSRuleList>>,
}

#[napi]
pub struct AnotherCSSStyleSheet {
  inner: Rc<RefCell<OwnedStyleSheet>>,
  rules: Reference<CSSRuleList>,
}

#[napi]
impl AnotherCSSStyleSheet {
  #[napi(getter)]
  pub fn rules(&self, mut env: Env) -> Result<Reference<CSSRuleList>> {
    env.with_scope(|scope| scope.clone_reference(&self.rules))
  }
}

#[napi]
impl CSSStyleSheet {
  #[napi(constructor)]
  pub fn new(name: String, rules: Vec<String>) -> Result<Self> {
    let inner = Rc::new(RefCell::new(OwnedStyleSheet { rules }));
    Ok(CSSStyleSheet {
      name,
      inner,
      rules: None,
    })
  }

  #[napi(getter)]
  pub fn rules(
    &mut self,
    mut env: Env,
    this: Reference<CSSStyleSheet>,
  ) -> Result<Reference<CSSRuleList>> {
    env.with_scope(|scope| {
      if let Some(rules) = &self.rules {
        return scope.clone_reference(rules);
      }

      let parent = scope.downgrade_reference(&this)?;
      let rules = scope.reference(CSSRuleList {
        owned: self.inner.clone(),
        parent,
      })?;

      self.rules = Some(scope.clone_reference(&rules)?);
      Ok(rules)
    })
  }

  #[napi]
  pub fn another_css_style_sheet(
    &self,
    mut env: Env,
  ) -> Result<ClassInitializer<AnotherCSSStyleSheet>> {
    env.with_scope(|scope| {
      Ok(ClassInitializer::from(AnotherCSSStyleSheet {
        inner: self.inner.clone(),
        rules: scope.clone_reference(self.rules.as_ref().unwrap())?,
      }))
    })
  }
}
