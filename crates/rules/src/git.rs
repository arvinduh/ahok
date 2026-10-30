//! `GitHook` rules: commit messages, staged diff size, and pushed branches.
//!
//! This module checks what git hands a hook against the rules. Reading the
//! message file, the staged diff and the pushed refs from git belongs to the
//! caller, which passes `None` when it couldn't; a rule with
//! `fail = "closed"` then blocks instead of letting the commit through.

/// A rule a commit or push broke.
#[derive(Debug)]
pub struct Violation<'a> {
  /// The rule's `id`.
  pub id: &'a str,
  /// Why, for the person or agent committing; the id when unset.
  pub reason: &'a str,
}

/// One file in the staged diff, as `git diff --numstat` counts it.
pub struct Change<'a> {
  /// Path relative to the repository root.
  pub path: &'a str,
  /// Lines added plus lines deleted; 0 for a binary file.
  pub lines: u32,
}

/// Checks a commit's `message` and staged `changes` against every
/// `commit-msg` rule; `changes` is `None` when the diff couldn't be read.
///
/// Comment lines are ignored, and merge, revert, `fixup!`, `squash!` and
/// `amend!` commits skip the subject rules: git writes those subjects.
#[must_use]
pub fn commit_msg<'a>(
  rules: &'a [crate::Rule],
  message: &str,
  changes: Option<&[Change]>,
  var_set: &dyn Fn(&str) -> bool,
) -> Vec<Violation<'a>> {
  let subject = message
    .lines()
    .find(|line| !line.starts_with('#') && !line.trim().is_empty())
    .unwrap_or_default();
  let generated = ["Merge ", "Revert ", "fixup! ", "squash! ", "amend! "]
    .iter()
    .any(|prefix| subject.starts_with(prefix));
  hook_rules(rules, crate::GitHook::CommitMsg)
    .filter(|rule| {
      !rule
        .allow_tag
        .as_deref()
        .is_some_and(|tag| message.contains(tag))
        && !rule.allow_env.as_deref().is_some_and(var_set)
    })
    .filter(|rule| {
      let bad_subject = rule
        .pattern
        .as_ref()
        .is_some_and(|pattern| !generated && !pattern.is_match(subject));
      let over_budget = rule.max_net_lines.is_some_and(|max| match changes {
        Some(changes) => counted_lines(rule, changes) > max,
        None => fails_closed(rule),
      });
      bad_subject || over_budget
    })
    .map(violation)
    .collect()
}

/// Checks the remote refs a push updates, such as `refs/heads/main`,
/// against every `pre-push` rule; `refs` is `None` when they couldn't be
/// read.
#[must_use]
pub fn pre_push<'a>(
  rules: &'a [crate::Rule],
  refs: Option<&[&str]>,
) -> Vec<Violation<'a>> {
  hook_rules(rules, crate::GitHook::PrePush)
    .filter(|rule| match refs {
      Some(refs) => refs.iter().any(|remote| {
        let branch = remote.strip_prefix("refs/heads/").unwrap_or(remote);
        rule.protected_branches.iter().any(|name| name == branch)
      }),
      None => fails_closed(rule),
    })
    .map(violation)
    .collect()
}

/// The `GitHook` rules that run in `hook`.
fn hook_rules(
  rules: &[crate::Rule],
  hook: crate::GitHook,
) -> impl Iterator<Item = &crate::Rule> {
  rules.iter().filter(move |rule| {
    matches!(rule.event, crate::Event::GitHook) && rule.hook == Some(hook)
  })
}

/// Lines `changes` add and delete, leaving out `rule`'s excluded paths.
fn counted_lines(rule: &crate::Rule, changes: &[Change]) -> u32 {
  changes
    .iter()
    .filter(|change| {
      !rule
        .exclude_globs
        .as_ref()
        .is_some_and(|globs| globs.is_match(change.path))
    })
    .map(|change| change.lines)
    .sum()
}

/// Whether `rule` blocks when its input couldn't be read.
fn fails_closed(rule: &crate::Rule) -> bool {
  matches!(rule.fail, Some(crate::Fail::Closed))
}

/// Reports `rule` as broken.
fn violation(rule: &crate::Rule) -> Violation<'_> {
  Violation {
    id: &rule.id,
    reason: rule.reason.as_deref().unwrap_or(&rule.id),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rules() -> Vec<crate::Rule> {
    crate::parse(include_str!("../tests/fixtures/rules/git.toml")).unwrap()
  }

  fn ids<'a>(violations: &[Violation<'a>]) -> Vec<&'a str> {
    violations.iter().map(|violation| violation.id).collect()
  }

  fn commit(message: &str, lines: u32, env: &str) -> Vec<String> {
    let rules = rules();
    let changes = [
      Change {
        path: "src/a.rs",
        lines,
      },
      Change {
        path: "Cargo.lock",
        lines: 500,
      },
    ];
    let var_set = |name: &str| name == env;
    let found = commit_msg(&rules, message, Some(&changes), &var_set);
    ids(&found).into_iter().map(str::to_owned).collect()
  }

  #[test]
  fn good_small_commit_passes() {
    assert!(commit("feat(git): add hooks\n\nWhy.", 50, "").is_empty());
  }

  #[test]
  fn bad_subject_and_big_diff_are_reported() {
    let found = commit("Added hooks.", 51, "");
    assert_eq!(found, ["conventional-commit-msg", "commit-diff-budget"]);
  }

  #[test]
  fn sweep_tag_and_env_exempt_their_rules() {
    assert!(commit("chore: format [sweep]", 900, "").is_empty());
    let found = commit("feat(x): big", 900, "ALLOW_LARGE_COMMIT");
    assert!(found.is_empty(), "{found:?}");
  }

  #[test]
  fn comments_and_merges_skip_the_subject_rule() {
    assert!(commit("# Please enter\nfix(a): add b\n", 1, "").is_empty());
    assert!(commit("Merge branch 'main' into x", 1, "").is_empty());
  }

  #[test]
  fn unreadable_diff_passes_unless_closed() {
    let rules = rules();
    let found = commit_msg(&rules, "fix(a): add b", None, &|_| false);
    assert!(found.is_empty(), "budget fails open");
    assert_eq!(ids(&pre_push(&rules, None)), ["no-push-default-branch"]);
  }

  #[test]
  fn push_to_a_protected_branch_is_reported() {
    let rules = rules();
    let found = pre_push(&rules, Some(&["refs/heads/main"]));
    assert_eq!(
      found[0].reason,
      "AGENTS.md §9: never push to the default branch."
    );
    assert!(pre_push(&rules, Some(&["refs/heads/feat-x"])).is_empty());
  }
}
