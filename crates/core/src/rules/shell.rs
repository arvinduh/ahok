//! Shell command lines split into simple commands.
//!
//! This module parses a command string with tree-sitter-bash and returns the
//! argv of every simple command in it, including those inside pipelines,
//! `&&` chains, subshells and `$(...)`. Matching argv against rules belongs
//! to `rules`.

use tree_sitter;
use tree_sitter_bash;

/// Returns the argv of each simple command in `line`, in source order.
///
/// Quotes are removed from plain and single-quoted words; expansions are
/// kept as written. Assignments and redirections are not part of argv.
///
/// # Panics
///
/// If the bundled bash grammar is incompatible with the tree-sitter
/// runtime, which is fixed at build time.
#[must_use]
pub fn commands(line: &str) -> Vec<Vec<String>> {
  let mut parser = tree_sitter::Parser::new();
  parser
    .set_language(&tree_sitter_bash::LANGUAGE.into())
    .expect("bash grammar matches the tree-sitter runtime");
  let Some(tree) = parser.parse(line, None) else {
    return Vec::new();
  };
  let mut out = Vec::new();
  let mut cursor = tree.walk();
  let mut stack = vec![tree.root_node()];
  while let Some(node) = stack.pop() {
    if node.kind() == "command" {
      out.push(argv(node, line));
    }
    let children: Vec<_> = node.children(&mut cursor).collect();
    stack.extend(children.into_iter().rev());
  }
  out
}

/// Collects the words of one `command` node.
fn argv(node: tree_sitter::Node, line: &str) -> Vec<String> {
  let mut cursor = node.walk();
  node
    .children(&mut cursor)
    .filter(|child| {
      matches!(
        child.kind(),
        "command_name"
          | "word"
          | "string"
          | "raw_string"
          | "concatenation"
          | "number"
          | "simple_expansion"
          | "expansion"
      )
    })
    .map(|child| unquote(&line[child.byte_range()]))
    .collect()
}

/// Strips one layer of matching outer quotes.
fn unquote(word: &str) -> String {
  for quote in ['\'', '"'] {
    if let Some(inner) = word
      .strip_prefix(quote)
      .and_then(|rest| rest.strip_suffix(quote))
    {
      return inner.to_owned();
    }
  }
  word.to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn simple_command() {
    assert_eq!(
      commands("git commit -m 'feat: x'"),
      [["git", "commit", "-m", "feat: x"]]
    );
  }

  #[test]
  fn chains_and_pipes() {
    assert_eq!(
      commands("cd a && git push origin main | tee log; echo done"),
      vec![
        vec!["cd", "a"],
        vec!["git", "push", "origin", "main"],
        vec!["tee", "log"],
        vec!["echo", "done"]
      ]
    );
  }

  #[test]
  fn nested_substitution_and_subshell() {
    let got = commands("echo $(git rev-parse HEAD) && (jj git push)");
    assert!(got.contains(&vec![
      "git".to_owned(),
      "rev-parse".to_owned(),
      "HEAD".to_owned()
    ]));
    assert!(got.contains(&vec![
      "jj".to_owned(),
      "git".to_owned(),
      "push".to_owned()
    ]));
  }

  #[test]
  fn skips_assignments_and_redirects() {
    assert_eq!(
      commands("FOO=1 cargo test > out.txt 2>&1"),
      [["cargo", "test"]]
    );
  }
}
