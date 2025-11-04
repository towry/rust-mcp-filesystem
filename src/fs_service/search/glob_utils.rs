use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::{ServiceError, ServiceResult};

pub(crate) fn compile_single_glob(
    pattern: &str,
    fallback: &str,
    case_insensitive: bool,
) -> ServiceResult<GlobSet> {
    let normalized = if pattern.trim().is_empty() {
        fallback.to_string()
    } else {
        pattern.to_string()
    };

    let mut builder = GlobSetBuilder::new();
    let mut glob_builder = GlobBuilder::new(&normalized);
    if case_insensitive {
        glob_builder.case_insensitive(true);
    }

    let glob = glob_builder.build().map_err(|err| {
        ServiceError::FromString(format!("Invalid glob pattern '{normalized}': {err}"))
    })?;

    builder.add(glob).build().map_err(|err| {
        ServiceError::FromString(format!(
            "Failed to build glob matcher for pattern '{normalized}': {err}"
        ))
    })
}

pub(crate) fn compile_exclude_glob(
    patterns: Option<&[String]>,
    case_insensitive: bool,
) -> ServiceResult<Option<GlobSet>> {
    let Some(patterns) = patterns else {
        return Ok(None);
    };

    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    let mut added = false;

    for pattern in patterns {
        if pattern.trim().is_empty() {
            continue;
        }

        let normalized = if pattern.contains('*') {
            pattern.strip_prefix('/').unwrap_or(pattern).to_owned()
        } else {
            format!("*{pattern}*")
        };

        let mut glob_builder = GlobBuilder::new(&normalized);
        if case_insensitive {
            glob_builder.case_insensitive(true);
        }

        let glob = glob_builder.build().map_err(|err| {
            ServiceError::FromString(format!("Invalid exclude glob pattern '{pattern}': {err}"))
        })?;

        builder.add(glob);
        added = true;
    }

    if !added {
        return Ok(None);
    }

    builder.build().map(Some).map_err(|err| {
        ServiceError::FromString(format!("Failed to build exclude glob patterns: {err}"))
    })
}
