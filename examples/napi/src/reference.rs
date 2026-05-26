use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

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
  pub fn remote(&self, #[napi(this)] this: ClassRef<JsRepo>) -> ClassInitializer<JsRemote> {
    ClassInitializer::from(JsRemote { repo: this })
  }
}

#[napi]
pub struct JsRemote {
  repo: ClassRef<JsRepo>,
}

#[napi]
impl JsRemote {
  #[napi(constructor)]
  pub fn new(repo: ClassRef<JsRepo>) -> Self {
    Self { repo }
  }

  #[napi]
  pub fn name(&self) -> Result<String> {
    let repo = self.repo.borrow()?;
    Ok(repo.inner.remote().name())
  }
}

struct OwnedStyleSheet {
  rules: Vec<String>,
}

#[napi]
pub struct CSSRuleList {
  owned: Rc<RefCell<OwnedStyleSheet>>,
  parent: WeakRef<Class<CSSStyleSheet>>,
}

#[napi]
impl CSSRuleList {
  #[napi]
  pub fn get_rules(&self) -> Vec<String> {
    self.owned.borrow().rules.to_vec()
  }

  #[napi(getter)]
  pub fn name(&self, #[napi(env)] mut env: Env) -> Result<Option<String>> {
    env.with_scope(|scope| {
      let Some(stylesheet) = self.parent.upgrade(scope)? else {
        return Ok(None);
      };
      let stylesheet_ref = stylesheet.as_class_local(scope)?;
      let stylesheet = stylesheet_ref.borrow()?;
      Ok(Some(stylesheet.name.clone()))
    })
  }

  #[napi(getter)]
  pub fn parent_style_sheet(&self, #[napi(env)] mut env: Env) -> Result<Ref<Class<CSSStyleSheet>>> {
    env.with_scope(|scope| {
      self.parent.upgrade(scope)?.ok_or_else(|| {
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
  rules: Option<Ref<Class<CSSRuleList>>>,
}

#[napi]
pub struct AnotherCSSStyleSheet {
  inner: Rc<RefCell<OwnedStyleSheet>>,
  rules: Ref<Class<CSSRuleList>>,
}

#[napi]
impl AnotherCSSStyleSheet {
  #[napi(getter)]
  pub fn rules(&self, #[napi(env)] mut env: Env) -> Result<Ref<Class<CSSRuleList>>> {
    env.with_scope(|scope| self.rules.clone(scope))
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
    #[napi(env)] mut env: Env,
    #[napi(this)] this: Ref<Class<CSSStyleSheet>>,
  ) -> Result<Ref<Class<CSSRuleList>>> {
    env.with_scope(|scope| {
      if let Some(rules) = &self.rules {
        return rules.clone(scope);
      }

      let parent = this.downgrade(scope)?;
      let rules = scope.reference(CSSRuleList {
        owned: self.inner.clone(),
        parent,
      })?;

      self.rules = Some(rules.clone(scope)?);
      Ok(rules)
    })
  }

  #[napi]
  pub fn another_css_style_sheet(
    &self,
    #[napi(env)] mut env: Env,
  ) -> Result<ClassInitializer<AnotherCSSStyleSheet>> {
    env.with_scope(|scope| {
      Ok(ClassInitializer::from(AnotherCSSStyleSheet {
        inner: self.inner.clone(),
        rules: self.rules.as_ref().unwrap().clone(scope)?,
      }))
    })
  }
}

#[napi]
pub struct SelfReferential {
  weak_self: OnceCell<WeakRef<Class<SelfReferential>>>,
  name: String,
}

#[napi]
impl SelfReferential {
  #[napi(constructor)]
  pub fn new(name: String) -> Self {
    SelfReferential {
      weak_self: OnceCell::new(),
      name,
    }
  }

  #[napi(post_init)]
  pub fn post_init(
    &self,
    #[napi(this)] this: Ref<Class<SelfReferential>>,
    #[napi(env)] mut env: Env,
  ) -> Result<()> {
    let weak = env.with_scope(|scope| this.downgrade(scope))?;
    let _ = self.weak_self.set(weak);
    Ok(())
  }

  #[napi(getter)]
  pub fn name(&self) -> &str {
    &self.name
  }

  #[napi]
  pub fn get_weak_name(&self, #[napi(env)] mut env: Env) -> Result<String> {
    let weak = self.weak_self.get().unwrap();
    env.with_scope(|scope| {
      let strong = weak
        .upgrade(scope)?
        .ok_or_else(|| Error::new(Status::GenericFailure, "Self reference expired".to_owned()))?;
      let bound = strong.as_class_local(scope)?;
      let this = bound.borrow()?;
      Ok(this.name.clone())
    })
  }
}

#[napi(subclass)]
pub struct PostInitBase {
  initialized: Cell<bool>,
}

impl PostInitBase {
  pub fn new() -> Self {
    PostInitBase {
      initialized: Cell::new(false),
    }
  }
}

#[napi]
impl PostInitBase {
  #[napi(post_init)]
  pub fn post_init(&self) {
    self.initialized.set(true);
  }

  #[napi]
  pub fn base_initialized(&self) -> bool {
    self.initialized.get()
  }
}

#[napi(extends = PostInitBase)]
pub struct PostInitChild {
  child_initialized: Cell<bool>,
  label: String,
}

#[napi]
impl PostInitChild {
  #[napi(constructor)]
  pub fn new(label: String) -> ClassInitializer<Self> {
    ClassInitializer::from_parent(
      ClassInitializer::from(PostInitBase::new()),
      Self {
        child_initialized: Cell::new(false),
        label,
      },
    )
  }

  #[napi(post_init)]
  pub fn post_init(&self) {
    self.child_initialized.set(true);
  }

  #[napi(getter)]
  pub fn label(&self) -> &str {
    &self.label
  }

  #[napi]
  pub fn child_initialized(&self) -> bool {
    self.child_initialized.get()
  }
}
