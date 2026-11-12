//! Tree-sitter based TypeScript declaration parsing.

mod declarations;
mod imports;
mod reexports;

use declarations::{collect_declarations, collect_unsupported_type_names};
use imports::{collect_imports, collect_inline_type_imports};
use reexports::{collect_attributed_type_wildcard_re_exports, collect_re_exports};

use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser};

use crate::{Result, TypeTuneError};

/// An exported TypeScript declaration found in one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDeclaration {
    /// Exported symbol name.
    pub symbol_name: String,
    /// TypeScript declaration kind compatible with existing JSON output.
    pub kind: String,
    /// UTF-16 source offset of the declaration including its export modifier.
    pub start_position: usize,
    /// Byte offset where the declaration including its export modifier starts.
    pub start_byte: usize,
    /// Byte offset where contiguous leading documentation comments begin.
    pub leading_comment_start_byte: usize,
    /// Byte offset immediately after the declaration node.
    pub end_byte: usize,
    /// Source text for the declaration including its export modifier.
    pub declaration_text: String,
    /// Leading documentation comments that move with the declaration.
    pub leading_comment_text: String,
    /// Whether the declaration is exported from its source file.
    pub is_exported: bool,
    /// Whether the declaration is the module's default export.
    pub is_default_export: bool,
    /// Type identifiers referenced by the declaration body.
    pub referenced_type_names: Vec<String>,
    /// Whether the declaration uses a `typeof` type query.
    pub has_type_query: bool,
}

impl ParsedDeclaration {
    pub(crate) fn stable_key(&self, source_path: &str) -> String {
        format!(
            "{source_path}#{}#{}#{}",
            self.kind, self.symbol_name, self.start_position
        )
    }
}

/// One named import and its module specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImport {
    /// Imported symbol name before any local alias.
    pub imported_name: String,
    /// Local binding name used in the referrer file.
    pub local_name: String,
    /// Whether this binding is the module's default import.
    pub is_default: bool,
    /// Whether this binding imports the module namespace.
    pub is_namespace: bool,
    /// Module specifier without quotes.
    pub module_specifier: String,
    /// Whether this binding is imported as type-only.
    pub is_type_only: bool,
    /// Byte offset where the quoted module specifier starts.
    pub module_specifier_start: usize,
    /// Byte offset immediately after the quoted module specifier.
    pub module_specifier_end: usize,
    /// Byte offset where the containing import statement starts.
    pub statement_start: usize,
    /// Byte offset immediately after the containing import statement.
    pub statement_end: usize,
}

/// A qualified type reference written as `import("module").Name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInlineTypeImport {
    /// Exported type name selected from the imported module.
    pub imported_name: String,
    /// Module specifier without quotes.
    pub module_specifier: String,
    /// Byte offset where the quoted module specifier starts.
    pub module_specifier_start: usize,
    /// Byte offset immediately after the quoted module specifier.
    pub module_specifier_end: usize,
}

/// One supported re-export and its module specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReExport {
    /// Name exported by the barrel file.
    pub exported_name: String,
    /// Original name re-exported from the source module.
    pub source_name: String,
    /// Whether this re-export exposes every source module export.
    pub is_wildcard: bool,
    /// Whether this wildcard uses the type-only `export type *` form.
    pub is_type_wildcard: bool,
    /// Whether this wildcard is exported under a type-only namespace name.
    pub is_type_namespace_wildcard: bool,
    /// Whether this wildcard is exported under a namespace name.
    pub is_namespace_wildcard: bool,
    /// Whether this specifier uses the inline `export { type Name }` form.
    pub is_inline_type_only: bool,
    /// Byte offset where the inline `type` keyword starts, when present.
    pub inline_type_keyword_start: Option<usize>,
    /// Byte offset immediately after the inline `type` keyword, when present.
    pub inline_type_keyword_end: Option<usize>,
    /// Module specifier without quotes.
    pub module_specifier: String,
    /// Byte offset where the quoted module specifier starts.
    pub module_specifier_start: usize,
    /// Byte offset immediately after the quoted module specifier.
    pub module_specifier_end: usize,
    /// Byte offset where the containing export statement starts.
    pub statement_start: usize,
    /// Byte offset immediately after the containing export statement.
    pub statement_end: usize,
}

/// Import binding references collected from one TypeScript source file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedImportUsage {
    /// Local imported binding names used outside import statements.
    pub binding_names: BTreeSet<String>,
    /// Referenced type member names grouped by imported namespace binding.
    pub namespace_members: BTreeMap<String, BTreeSet<String>>,
}

/// Parsed import declarations and their source-level usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedImportAnalysis {
    /// Import declarations in source order.
    pub imports: Vec<ParsedImport>,
    /// Inline `import("module").Name` type references in source order.
    pub inline_type_imports: Vec<ParsedInlineTypeImport>,
    /// Usage of those import bindings outside import statements.
    pub usage: ParsedImportUsage,
}

/// Parsed declarations, imports, import usage, and re-exports from one source file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSource {
    /// Supported type declarations in source order.
    pub declarations: Vec<ParsedDeclaration>,
    /// Parsed imports and their binding usage.
    pub import_analysis: ParsedImportAnalysis,
    /// Supported re-exports in source order.
    pub re_exports: Vec<ParsedReExport>,
    /// Same-file class, enum, or namespace names that cannot move as type dependencies.
    pub unsupported_type_names: BTreeSet<String>,
}

impl ParsedSource {
    /// Iterates over import, inline type import, and re-export specifiers in that order.
    pub(crate) fn module_specifiers(&self) -> impl Iterator<Item = &str> {
        self.import_analysis
            .imports
            .iter()
            .map(|import| import.module_specifier.as_str())
            .chain(
                self.import_analysis
                    .inline_type_imports
                    .iter()
                    .map(|import| import.module_specifier.as_str()),
            )
            .chain(
                self.re_exports
                    .iter()
                    .map(|export| export.module_specifier.as_str()),
            )
    }
}

/// Finds exported type aliases and interfaces without executing source code.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_exported_declarations(source: &str) -> Result<Vec<ParsedDeclaration>> {
    Ok(parse_type_declarations(source)?
        .into_iter()
        .filter(|declaration| declaration.is_exported && !declaration.is_default_export)
        .collect())
}

/// Finds supported type aliases and interfaces, including non-exported declarations.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_type_declarations(source: &str) -> Result<Vec<ParsedDeclaration>> {
    Ok(parse_source(source)?.declarations)
}

/// Parses all native scan data from TypeScript source once.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_source(source: &str) -> Result<ParsedSource> {
    let tree = parse_tree(source)?;
    Ok(parsed_source_from_tree(&tree, source))
}

/// TypeScript dialect used for one embedded script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeScriptDialect {
    /// `.ts` script content.
    TypeScript,
    /// `.tsx` script content.
    Tsx,
}

/// Parses source with one dialect and returns nothing when the tree contains syntax errors.
///
/// Vue script extraction uses this so an ambiguous or broken block is rejected instead of being
/// repaired by the TypeScript-to-TSX fallback.
///
/// # Errors
///
/// Returns an error when the parser cannot be initialized.
pub(crate) fn parse_exact_dialect(
    source: &str,
    dialect: TypeScriptDialect,
) -> Result<Option<ParsedSource>> {
    let tree = parse_tree_with_dialect(source, dialect)?;
    if tree.root_node().has_error() {
        return Ok(None);
    }
    Ok(Some(parsed_source_from_tree(&tree, source)))
}

fn parsed_source_from_tree(tree: &tree_sitter::Tree, source: &str) -> ParsedSource {
    let mut declarations = Vec::new();
    collect_declarations(
        tree.root_node(),
        source.as_bytes(),
        false,
        false,
        None,
        &mut declarations,
    );
    let mut imports = Vec::new();
    collect_imports(tree.root_node(), source.as_bytes(), &mut imports);
    let mut inline_type_imports = Vec::new();
    collect_inline_type_imports(
        tree.root_node(),
        source.as_bytes(),
        &mut inline_type_imports,
    );
    let mut usage = ParsedImportUsage::default();
    collect_import_usage(tree.root_node(), source.as_bytes(), false, &mut usage);
    let mut re_exports = Vec::new();
    collect_re_exports(tree.root_node(), source.as_bytes(), &mut re_exports);
    collect_attributed_type_wildcard_re_exports(source, &mut re_exports);
    let mut unsupported_type_names = BTreeSet::new();
    collect_unsupported_type_names(
        tree.root_node(),
        source.as_bytes(),
        &mut unsupported_type_names,
    );
    ParsedSource {
        declarations,
        import_analysis: ParsedImportAnalysis {
            imports,
            inline_type_imports,
            usage,
        },
        re_exports,
        unsupported_type_names,
    }
}

/// Returns whether `source` uses any of `type_names` outside import statements.
///
/// Named re-exports use plain identifiers rather than type identifiers, so they are not treated
/// as remaining usages. Value and type annotations such as `const label: Shared` are detected.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn source_has_type_usage(source: &str, type_names: &BTreeSet<String>) -> Result<bool> {
    if type_names.is_empty() || source.trim().is_empty() {
        return Ok(false);
    }
    let tree = parse_tree(source)?;
    Ok(collect_type_usage(
        tree.root_node(),
        source.as_bytes(),
        type_names,
        false,
    ))
}

/// Parses named imports from TypeScript source.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_named_imports(source: &str) -> Result<Vec<ParsedImport>> {
    Ok(parse_import_analysis(source)?.imports)
}

/// Parses import declarations and their usage from TypeScript source once.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_import_analysis(source: &str) -> Result<ParsedImportAnalysis> {
    Ok(parse_source(source)?.import_analysis)
}

/// Parses supported re-exports from TypeScript source.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_named_re_exports(source: &str) -> Result<Vec<ParsedReExport>> {
    Ok(parse_source(source)?.re_exports)
}

/// Finds type members referenced through one imported namespace binding.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_namespace_type_members(source: &str, namespace: &str) -> Result<Vec<String>> {
    Ok(parse_import_usage(source)?
        .namespace_members
        .get(namespace)
        .map_or_else(Vec::new, |members| members.iter().cloned().collect()))
}

/// Collects import binding usage from TypeScript source in one parser traversal.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn parse_import_usage(source: &str) -> Result<ParsedImportUsage> {
    Ok(parse_import_analysis(source)?.usage)
}

/// Returns no analysis when syntax errors make import cleanup uncertain.
pub(crate) fn parse_import_usage_for_cleanup(source: &str) -> Result<Option<ParsedImportUsage>> {
    let tree = parse_tree(source)?;
    if tree.root_node().has_error() {
        return Ok(None);
    }
    let mut usage = ParsedImportUsage::default();
    collect_import_usage(tree.root_node(), source.as_bytes(), false, &mut usage);
    Ok(Some(usage))
}

/// Checks whether a local import binding occurs outside import statements.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub fn has_binding_usage(source: &str, binding: &str) -> Result<bool> {
    Ok(parse_import_usage(source)?.binding_names.contains(binding))
}

fn collect_import_usage(
    node: Node<'_>,
    source: &[u8],
    inside_import: bool,
    usage: &mut ParsedImportUsage,
) {
    let inside_import = inside_import || node.kind() == "import_statement";
    if !inside_import
        && matches!(node.kind(), "identifier" | "type_identifier")
        && let Ok(binding) = node.utf8_text(source)
    {
        usage.binding_names.insert(binding.into());
    }
    if !inside_import
        && node.kind() == "nested_type_identifier"
        && let Some(module) = node.child_by_field_name("module")
        && let Ok(namespace) = module.utf8_text(source)
        && let Some(name) = node.child_by_field_name("name")
        && let Ok(member) = name.utf8_text(source)
    {
        usage
            .namespace_members
            .entry(namespace.into())
            .or_default()
            .insert(member.into());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_usage(child, source, inside_import, usage);
    }
}

fn parse_tree(source: &str) -> Result<tree_sitter::Tree> {
    let tree = parse_tree_with_dialect(source, TypeScriptDialect::TypeScript)?;
    if !tree.root_node().has_error() {
        return Ok(tree);
    }
    parse_tree_with_dialect(source, TypeScriptDialect::Tsx)
}

fn parse_tree_with_dialect(source: &str, dialect: TypeScriptDialect) -> Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let (language, label) = match dialect {
        TypeScriptDialect::TypeScript => {
            (tree_sitter_typescript::LANGUAGE_TYPESCRIPT, "TypeScript")
        }
        TypeScriptDialect::Tsx => (tree_sitter_typescript::LANGUAGE_TSX, "TSX"),
    };
    parser.set_language(&language.into()).map_err(|error| {
        TypeTuneError::new(format!("Could not initialize {label} parser: {error}"))
    })?;
    parser
        .parse(source, None)
        .ok_or_else(|| TypeTuneError::new(format!("Could not parse {label} source.")))
}

fn first_string_child(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "string" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_string_child)
}

fn collect_type_usage(
    node: Node<'_>,
    source: &[u8],
    type_names: &BTreeSet<String>,
    inside_import: bool,
) -> bool {
    let inside_import = inside_import || node.kind() == "import_statement";
    if !inside_import {
        if node.kind() == "nested_type_identifier"
            && let Some(module) = node.child_by_field_name("module")
            && let Ok(module_name) = module.utf8_text(source)
            && let Some(namespace_name) = module_name.split('.').next()
            && type_names.contains(namespace_name)
        {
            return true;
        }
        if node.kind() == "type_identifier"
            && let Ok(name) = node.utf8_text(source)
            && type_names.contains(name)
        {
            return true;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if collect_type_usage(child, source, type_names, inside_import) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedImport, ParsedReExport, has_binding_usage, parse_exported_declarations,
        parse_import_analysis, parse_named_imports, parse_named_re_exports,
        parse_namespace_type_members, parse_source, parse_type_declarations,
    };

    #[test]
    fn treats_typeof_import_as_extractable_type_reference() {
        let parsed = parse_source("export type Shared = typeof import('./external').value;")
            .expect("parses");
        let declaration = parsed.declarations.first().expect("declaration");
        assert!(!declaration.has_type_query);
    }

    #[test]
    fn detects_type_usage_in_value_annotations() {
        let names = std::collections::BTreeSet::from(["Shared".into()]);
        assert!(
            super::source_has_type_usage("export const label: Shared = {};", &names,)
                .expect("parses")
        );
        assert!(
            !super::source_has_type_usage("export const label = 'shared';", &names,)
                .expect("parses")
        );
        assert!(
            !super::source_has_type_usage("export type { Shared } from './types/src';", &names,)
                .expect("parses")
        );
        assert!(
            !super::source_has_type_usage("import type { Shared } from './types/src';", &names,)
                .expect("parses")
        );
    }

    #[test]
    fn finds_only_exported_supported_declarations() {
        let declarations = parse_exported_declarations(
            "export type Shared = {}; export interface User {} type Local = {};",
        )
        .expect("parses");
        assert_eq!(
            declarations
                .into_iter()
                .map(|value| value.symbol_name)
                .collect::<Vec<_>>(),
            ["Shared", "User"]
        );
    }

    #[test]
    fn finds_declarations_in_tsx_source() {
        let declarations = parse_exported_declarations(
            "export type Props = { id: string }; export const View = () => <div />;",
        )
        .expect("parses");
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].symbol_name, "Props");
    }

    #[test]
    fn excludes_default_export_interfaces() {
        let declarations = parse_exported_declarations(
            "export default interface Hidden {} export interface Visible {}",
        )
        .expect("parses");
        assert_eq!(
            declarations
                .iter()
                .map(|declaration| declaration.symbol_name.as_str())
                .collect::<Vec<_>>(),
            ["Visible"]
        );
    }

    #[test]
    fn records_export_start_as_a_utf16_offset() {
        let declarations =
            parse_exported_declarations("// 😀\nexport type Shared = {};").expect("parses");
        assert_eq!(declarations[0].start_position, 6);
        assert_eq!(declarations[0].start_byte, 8);
    }

    #[test]
    fn leaves_a_file_banner_when_a_blank_line_separates_it() {
        let declarations = parse_exported_declarations(
            "/** File banner. */\n\n/** Shared contract. */\nexport type Shared = {};",
        )
        .expect("parses");
        assert_eq!(
            declarations[0].leading_comment_text,
            "/** Shared contract. */"
        );
    }

    #[test]
    fn collects_contiguous_leading_documentation_comments() {
        let declarations = parse_exported_declarations(
            "/** Shared contract. */\n// Kept with the declaration.\nexport type Shared = {};",
        )
        .expect("parses");
        assert_eq!(declarations[0].leading_comment_start_byte, 0);
        assert_eq!(
            declarations[0].leading_comment_text,
            "/** Shared contract. */\n// Kept with the declaration."
        );
        assert_eq!(declarations[0].start_byte, 54);
    }

    #[test]
    fn collects_non_exported_type_dependencies() {
        let declarations =
            parse_type_declarations("type Detail = { id: string }; export type Shared = Detail;")
                .expect("parses");
        assert!(
            declarations
                .iter()
                .any(|declaration| !declaration.is_exported)
        );
        assert_eq!(
            declarations
                .iter()
                .find(|declaration| declaration.symbol_name == "Shared")
                .expect("finds shared")
                .referenced_type_names,
            ["Detail", "Shared"]
        );
    }

    #[test]
    fn collects_namespace_type_dependencies() {
        let declarations =
            parse_type_declarations("export type Shared = External.Value;").expect("parses");
        assert_eq!(
            declarations
                .iter()
                .find(|declaration| declaration.symbol_name == "Shared")
                .expect("finds shared")
                .referenced_type_names,
            ["External", "Shared", "Value"]
        );
    }

    #[test]
    fn collects_namespace_type_dependencies_in_interface_heritage() {
        let declarations = parse_type_declarations(
            "export interface Shared extends ExternalTypes.Base { item: ExternalTypes.Item }",
        )
        .expect("parses");
        assert_eq!(
            declarations
                .iter()
                .find(|declaration| declaration.symbol_name == "Shared")
                .expect("finds shared")
                .referenced_type_names,
            ["Base", "ExternalTypes", "Item", "Shared"]
        );
    }

    #[test]
    fn finds_members_referenced_through_a_type_namespace() {
        assert_eq!(
            parse_namespace_type_members(
                "import type * as Types from './types'; export type Value = Types.Schema | Types.Detail;",
                "Types",
            )
            .expect("parses"),
            ["Detail", "Schema"]
        );
    }

    #[test]
    fn collects_import_usage_in_one_parse() {
        let analysis = parse_import_analysis(
            "import type { Shared } from './shared'; import type * as Types from './types'; type View = Shared | Types.Schema;",
        )
        .expect("parses");
        assert_eq!(analysis.imports.len(), 2);
        let usage = analysis.usage;
        assert!(usage.binding_names.contains("Shared"));
        assert!(usage.binding_names.contains("Types"));
        assert_eq!(
            usage.namespace_members.get("Types"),
            Some(&std::collections::BTreeSet::from(["Schema".into()]))
        );
    }

    #[test]
    fn import_cleanup_declines_to_analyze_invalid_syntax() {
        assert!(
            super::parse_import_usage_for_cleanup("import type { A } from './a';\nconst x = (")
                .expect("parses conservatively")
                .is_none()
        );
    }

    #[test]
    fn collects_scan_data_from_one_source_parse() {
        let parsed = parse_source(
            "import type { Detail } from './detail'; export type Shared = Detail; export { Detail } from './detail';",
        )
        .expect("parses");
        assert_eq!(parsed.declarations.len(), 1);
        assert_eq!(parsed.import_analysis.imports.len(), 1);
        assert_eq!(parsed.re_exports.len(), 1);
    }

    #[test]
    fn iterates_all_parsed_module_specifiers_by_kind() {
        let parsed = parse_source(
            "export { A } from './barrel'; export type B = import('./inline').B; import { C } from './import';",
        )
        .expect("parses");
        assert_eq!(
            parsed.module_specifiers().collect::<Vec<_>>(),
            ["./import", "./inline", "./barrel"]
        );
    }

    #[test]
    fn finds_inline_type_imports() {
        let analysis =
            parse_import_analysis("export type App = import('./shared').Shared;").expect("parses");
        assert_eq!(
            analysis.inline_type_imports,
            [super::ParsedInlineTypeImport {
                imported_name: "Shared".into(),
                module_specifier: "./shared".into(),
                module_specifier_start: 25,
                module_specifier_end: 35,
            }]
        );
    }

    #[test]
    fn ignores_runtime_dynamic_imports() {
        let analysis = parse_import_analysis("const load = import('./shared').then(() => {});")
            .expect("parses");
        assert!(analysis.inline_type_imports.is_empty());
    }

    #[test]
    fn finds_named_type_imports() {
        let imports = parse_named_imports("import type { Shared as Local } from './shared';")
            .expect("parses");
        assert_eq!(
            imports,
            [ParsedImport {
                imported_name: "Shared".into(),
                local_name: "Local".into(),
                is_default: false,
                is_namespace: false,
                module_specifier: "./shared".into(),
                is_type_only: true,
                module_specifier_start: 37,
                module_specifier_end: 47,
                statement_start: 0,
                statement_end: 48,
            }]
        );
    }

    #[test]
    fn records_inline_type_only_imports() {
        let imports =
            parse_named_imports("import { type Shared } from './shared';").expect("parses");
        assert!(imports[0].is_type_only);
    }

    #[test]
    fn finds_default_type_imports() {
        let imports =
            parse_named_imports("import type External from './external';").expect("parses");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "default");
        assert_eq!(imports[0].local_name, "External");
        assert!(imports[0].is_default);
        assert!(!imports[0].is_namespace);
        assert!(imports[0].is_type_only);
    }

    #[test]
    fn finds_namespace_type_imports() {
        let imports =
            parse_named_imports("import type * as External from './external';").expect("parses");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].imported_name, "*");
        assert_eq!(imports[0].local_name, "External");
        assert!(!imports[0].is_default);
        assert!(imports[0].is_namespace);
        assert!(imports[0].is_type_only);
    }

    #[test]
    fn finds_named_re_exports() {
        assert_eq!(
            parse_named_re_exports("export type { Shared as PublicShared } from './shared';")
                .expect("parses"),
            [ParsedReExport {
                exported_name: "PublicShared".into(),
                source_name: "Shared".into(),
                is_wildcard: false,
                is_type_wildcard: false,
                is_type_namespace_wildcard: false,
                is_namespace_wildcard: false,
                is_inline_type_only: false,
                inline_type_keyword_start: None,
                inline_type_keyword_end: None,
                module_specifier: "./shared".into(),
                module_specifier_start: 44,
                module_specifier_end: 54,
                statement_start: 0,
                statement_end: 55,
            }]
        );
    }

    #[test]
    fn finds_attributed_type_re_exports() {
        let exports = parse_named_re_exports(
            "export type { Shared as PublicShared, Other } from './shared.js' with { 'resolution-mode': 'import' };",
        )
        .expect("parses");
        assert_eq!(exports.len(), 2);
        assert_eq!(exports[0].source_name, "Shared");
        assert_eq!(exports[0].exported_name, "PublicShared");
        assert_eq!(exports[1].source_name, "Other");
        assert_eq!(exports[1].exported_name, "Other");
        assert_eq!(exports[0].module_specifier, "./shared.js");

        let wildcard = parse_named_re_exports(
            "export type * from './shared.js' with { 'resolution-mode': 'import' };",
        )
        .expect("parses");
        assert_eq!(wildcard.len(), 1);
        assert!(wildcard[0].is_wildcard);
        assert!(wildcard[0].is_type_wildcard);

        let namespace = parse_named_re_exports(
            "export type * as Types from './shared.js' with { 'resolution-mode': 'import' };",
        )
        .expect("parses");
        assert_eq!(namespace.len(), 1);
        assert_eq!(namespace[0].exported_name, "Types");
        assert!(namespace[0].is_type_namespace_wildcard);
    }

    #[test]
    fn records_inline_type_only_re_exports() {
        let exports =
            parse_named_re_exports("export { type Shared as PublicShared } from './shared';")
                .expect("parses");
        assert_eq!(exports.len(), 1);
        assert!(exports[0].is_inline_type_only);
        assert_eq!(exports[0].inline_type_keyword_start, Some(9));
        assert_eq!(exports[0].inline_type_keyword_end, Some(13));
    }

    #[test]
    fn distinguishes_imports_from_binding_usage() {
        assert!(
            !has_binding_usage("import type { Shared } from './shared';", "Shared")
                .expect("parses")
        );
        assert!(
            has_binding_usage(
                "import type { Shared } from './shared'; type View = Shared;",
                "Shared"
            )
            .expect("parses")
        );
    }

    #[test]
    fn finds_wildcard_re_exports() {
        let exports = parse_named_re_exports("export * from './shared';").expect("parses");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].exported_name, "*");
        assert_eq!(exports[0].source_name, "*");
        assert!(exports[0].is_wildcard);
        assert!(!exports[0].is_type_wildcard);
        assert!(!exports[0].is_type_namespace_wildcard);
        assert!(!exports[0].is_namespace_wildcard);
    }

    #[test]
    fn finds_type_wildcard_re_exports() {
        let exports = parse_named_re_exports("export type * from './shared';").expect("parses");
        assert_eq!(exports.len(), 1);
        assert!(exports[0].is_wildcard);
        assert!(exports[0].is_type_wildcard);
        assert!(!exports[0].is_type_namespace_wildcard);
        assert!(!exports[0].is_namespace_wildcard);
    }

    #[test]
    fn finds_type_namespace_wildcard_re_exports() {
        let exports =
            parse_named_re_exports("export type * as Types from './shared';").expect("parses");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].exported_name, "Types");
        assert!(exports[0].is_type_namespace_wildcard);
        assert!(exports[0].is_namespace_wildcard);
    }

    #[test]
    fn finds_namespace_wildcard_re_exports() {
        let exports = parse_named_re_exports("export * as Types from './shared';").expect("parses");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].exported_name, "Types");
        assert!(exports[0].is_namespace_wildcard);
        assert!(!exports[0].is_type_namespace_wildcard);
    }
}
