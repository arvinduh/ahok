//! Serde field parsers that compile patterns while a file loads.
//!
//! This module owns turning pattern strings into matchers, so a bad regex or
//! glob fails loading with its file. The schemas that use them belong to
//! `rules` and `check`.

use globset;
use regex;
use serde;

/// Compiles a regex field, so a bad pattern fails loading with its file.
pub fn pattern<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> Result<Option<regex::Regex>, D::Error> {
  let pattern = <String as serde::Deserialize>::deserialize(deserializer)?;
  regex::Regex::new(&pattern)
    .map(Some)
    .map_err(<D::Error as serde::de::Error>::custom)
}

/// Compiles a glob list into one matcher, failing loading on a bad glob.
pub fn globs<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> Result<Option<globset::GlobSet>, D::Error> {
  let globs = <Vec<String> as serde::Deserialize>::deserialize(deserializer)?;
  let mut set = globset::GlobSetBuilder::new();
  for glob in &globs {
    set.add(globset::Glob::new(glob).map_err(serde::de::Error::custom)?);
  }
  set.build().map(Some).map_err(serde::de::Error::custom)
}
