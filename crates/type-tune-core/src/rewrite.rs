//! Conservative native extraction rewrite planning.

use std::collections::BTreeMap;

use crate::{Result, model::Candidate, module_path::PackageAliasScopes};

mod barrel;
mod compatibility;
mod destination;
mod format;
mod imports;
mod mixed_imports;
mod options;
mod planning;
mod reexports;
mod referrer_edits;
mod referrers;
mod resolution;
mod source;
mod validation;
mod vue_scripts;

pub use options::{CompatibilityReexportQuoteStyle, RewriteOptions};

/// type-only re-export. An import is added only when remaining source code still references a
/// moved type. Private local dependencies that remaining declarations still use stay in the source
/// and are copied into the destination. Private dependencies used only by the moved type move with
/// it, even when the source file keeps other code.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported import strategies, or unsafe
/// source files that would retain declarations after moving the selected types.
pub fn plan_extraction_rewrite(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_aliases(
        candidates,
        sources,
        None,
        &BTreeMap::new(),
        CompatibilityReexportQuoteStyle::Apply,
    )
}

/// Plans source and referrer changes while preserving configured TypeScript path aliases.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
pub fn plan_extraction_rewrite_with_aliases(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    compatibility_quote_style: CompatibilityReexportQuoteStyle,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_aliases_and_root_dirs(
        candidates,
        sources,
        base_url,
        path_aliases,
        &[],
        compatibility_quote_style,
    )
}

/// Plans source and referrer changes while preserving TypeScript path aliases and `rootDirs`.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
pub fn plan_extraction_rewrite_with_aliases_and_root_dirs(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    root_dirs: &[String],
    compatibility_quote_style: CompatibilityReexportQuoteStyle,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_resolution_options(
        candidates,
        sources,
        base_url,
        path_aliases,
        root_dirs,
        &[],
        compatibility_quote_style,
    )
}

/// Plans source and referrer changes while preserving TypeScript module resolution options.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
pub fn plan_extraction_rewrite_with_resolution_options(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    root_dirs: &[String],
    module_suffixes: &[String],
    compatibility_quote_style: CompatibilityReexportQuoteStyle,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_scoped_resolution_options(
        candidates,
        sources,
        base_url,
        path_aliases,
        &PackageAliasScopes::new(),
        root_dirs,
        module_suffixes,
        compatibility_quote_style,
    )
}

/// Plans source and referrer changes with package aliases scoped to their source packages.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
#[allow(clippy::too_many_arguments)]
pub fn plan_extraction_rewrite_with_scoped_resolution_options(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    package_aliases: &PackageAliasScopes,
    root_dirs: &[String],
    module_suffixes: &[String],
    compatibility_quote_style: CompatibilityReexportQuoteStyle,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_scoped_resolution_options_and_extension(
        candidates,
        sources,
        base_url,
        path_aliases,
        package_aliases,
        root_dirs,
        module_suffixes,
        "",
        compatibility_quote_style,
    )
}

/// Plans source and referrer changes with package aliases and a default generated extension.
///
/// Use this when the compiler requires explicit JavaScript extensions for generated relative
/// module specifiers, such as NodeNext ESM projects.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
#[allow(clippy::too_many_arguments)]
pub fn plan_extraction_rewrite_with_scoped_resolution_options_and_extension(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    base_url: Option<&str>,
    path_aliases: &BTreeMap<String, Vec<String>>,
    package_aliases: &PackageAliasScopes,
    root_dirs: &[String],
    module_suffixes: &[String],
    default_module_specifier_extension: &str,
    compatibility_quote_style: CompatibilityReexportQuoteStyle,
) -> Result<BTreeMap<String, String>> {
    plan_extraction_rewrite_with_options(
        candidates,
        sources,
        RewriteOptions {
            base_url,
            path_aliases,
            package_aliases,
            root_dirs,
            module_suffixes,
            default_module_specifier_extension,
            compatibility_quote_style,
        },
    )
}

/// Plans source and referrer changes using a single module-resolution options value.
///
/// # Errors
///
/// Returns an error for conflicting destinations, unsupported strategies, or unsafe source files.
pub fn plan_extraction_rewrite_with_options(
    candidates: &[Candidate],
    sources: &BTreeMap<String, String>,
    options: RewriteOptions<'_>,
) -> Result<BTreeMap<String, String>> {
    planning::plan_rewrite(candidates, sources, options)
}

#[cfg(test)]
mod tests;
