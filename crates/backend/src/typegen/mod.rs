use std::fmt::{self, Display, Formatter};

use convert_case::Case;
use quote::ToTokens;
use syn::{Member, Pat};

use crate::util::to_case;

pub mod tokens;

#[derive(Default, Debug)]
pub struct JSDoc {
  blocks: Vec<Vec<String>>,
}

/// Formats a JavaScript property name, adding quotes if it contains special characters
/// or starts with a digit that would make it an invalid identifier.
pub fn format_js_property_name(js_name: &str) -> String {
  let starts_with_digit = js_name.chars().next().is_some_and(|c| c.is_ascii_digit());

  let has_invalid_chars = js_name.chars().any(|c| {
    matches!(
      c,
      '-'
        | ':'
        | ' '
        | '.'
        | '['
        | ']'
        | '@'
        | '#'
        | '$'
        | '%'
        | '^'
        | '&'
        | '*'
        | '('
        | ')'
        | '+'
        | '='
        | '{'
        | '}'
        | '|'
        | '\\'
        | ';'
        | '\''
        | '"'
        | '<'
        | '>'
        | ','
        | '?'
        | '/'
        | '~'
        | '`'
        | '!'
    )
  });

  if starts_with_digit || has_invalid_chars {
    format!("'{js_name}'")
  } else {
    js_name.to_string()
  }
}

pub fn gen_ts_func_arg_pub(pat: &Pat) -> String {
  gen_ts_func_arg(pat)
}

fn gen_ts_func_arg(pat: &Pat) -> String {
  match pat {
    Pat::Struct(s) => format!(
      "{{ {} }}",
      s.fields
        .iter()
        .map(|field| {
          let member_str = match &field.member {
            Member::Named(ident) => ident.to_string(),
            Member::Unnamed(index) => format!("field{}", index.index),
          };
          let nested_str = gen_ts_func_arg(&field.pat);
          if member_str == nested_str {
            to_case(member_str, Case::Camel)
          } else {
            format!("{}: {}", to_case(member_str, Case::Camel), nested_str)
          }
        })
        .collect::<Vec<_>>()
        .join(", ")
    ),
    Pat::TupleStruct(ts) => format!(
      "{{ {} }}",
      ts.elems
        .iter()
        .enumerate()
        .map(|(index, elem)| {
          let member_str = format!("field{index}");
          let nested_str = gen_ts_func_arg(elem);
          format!("{member_str}: {nested_str}")
        })
        .collect::<Vec<_>>()
        .join(", ")
    ),
    Pat::Tuple(t) => format!(
      "[{}]",
      t.elems
        .iter()
        .map(gen_ts_func_arg)
        .collect::<Vec<_>>()
        .join(", ")
    ),
    Pat::Wild(_) => "_".to_string(),
    _ => to_case(pat.to_token_stream().to_string(), Case::Camel),
  }
}

impl JSDoc {
  pub fn new<I, S>(initial_lines: I) -> JSDoc
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    let block = Self::cleanup_lines(initial_lines);
    if block.is_empty() {
      return Self { blocks: vec![] };
    }

    Self {
      blocks: vec![block],
    }
  }

  pub fn add_block<I, S>(&mut self, lines: I)
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    let v: Vec<String> = Self::cleanup_lines(lines);

    if !v.is_empty() {
      self.blocks.push(v);
    }
  }

  fn cleanup_lines<I, S>(lines: I) -> Vec<String>
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    let raw: Vec<String> = lines.into_iter().map(Into::into).collect();

    if let (Some(first_non_blank), Some(last_non_blank)) = (
      raw.iter().position(|l| !l.trim().is_empty()),
      raw.iter().rposition(|l| !l.trim().is_empty()),
    ) {
      let min_indent = raw[first_non_blank..=last_non_blank]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

      raw[first_non_blank..=last_non_blank]
        .iter()
        .map(|l| {
          if l.trim().is_empty() {
            String::new()
          } else if l.len() >= min_indent {
            l[min_indent..].to_owned()
          } else {
            l.to_owned()
          }
        })
        .collect()
    } else {
      Vec::new()
    }
  }
}

impl Display for JSDoc {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    if self.blocks.is_empty() {
      return Ok(());
    }

    fn escape_comment_close(s: &str) -> String {
      s.replace("*/", "*\\/")
    }

    if self.blocks.len() == 1 && self.blocks[0].len() == 1 {
      return writeln!(f, "/** {} */", escape_comment_close(&self.blocks[0][0]));
    }

    writeln!(f, "/**")?;
    for (i, block) in self.blocks.iter().enumerate() {
      for line in block {
        writeln!(f, " * {}", escape_comment_close(line))?;
      }
      if i + 1 != self.blocks.len() {
        writeln!(f, " *")?;
      }
    }
    writeln!(f, " */")
  }
}
