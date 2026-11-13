//! Read-only extraction of TypeScript from Vue single-file components.
//!
//! The extractor walks top-level SFC blocks and keeps only supported `<script>` bodies. Template
//! and style contents stay out of the TypeScript parser. Script byte offsets match the original
//! file so a later rewrite can map edits without reparsing the component.

use crate::{
    Result,
    model::{ANALYSIS_LIMITATION_UNRESOLVED_VUE, AnalysisLimitation},
    parser::{ParsedSource, TypeScriptDialect, parse_exact_dialect},
    source_role::{SourceRole, source_role},
};

/// A project file parsed for referrer analysis.
pub(crate) struct AnalyzedSource {
    /// Project-relative path.
    pub(crate) path: String,
    /// TypeScript analysis. Vue script offsets are positions in the original file.
    pub(crate) parsed: ParsedSource,
    /// Inclusive-exclusive byte ranges of supported Vue script bodies in the original file.
    pub(crate) script_ranges: Vec<(usize, usize)>,
    /// Whether the rewrite pipeline may edit this file.
    pub(crate) role: SourceRole,
}

/// Result of reading one discovered project file.
pub(crate) enum SourceParse {
    /// TypeScript or a supported Vue script block.
    Analyzed(AnalyzedSource),
    /// Vue component with no inline TypeScript to analyze.
    NoTypeScript,
    /// Vue component whose script content cannot be trusted.
    Unsupported(AnalysisLimitation),
}

/// Parses a project file, extracting Vue script blocks instead of treating `.vue` as TypeScript.
///
/// # Errors
///
/// Returns an error when the TypeScript parser cannot be initialized.
pub(crate) fn parse_analysis_source(path: &str, source: &str) -> Result<SourceParse> {
    if source_role(path) != SourceRole::ReadOnlyVue {
        return Ok(SourceParse::Analyzed(AnalyzedSource {
            path: path.to_owned(),
            parsed: crate::parser::parse_source(source)?,
            script_ranges: Vec::new(),
            role: SourceRole::WritableTypeScript,
        }));
    }
    Ok(match extract_vue_typescript(path, source) {
        VueExtract::NoTypeScript => SourceParse::NoTypeScript,
        VueExtract::Unsupported(limitation) => SourceParse::Unsupported(limitation),
        VueExtract::TypeScript {
            embedded,
            dialect,
            script_ranges,
        } => match parse_exact_dialect(&embedded, dialect)? {
            Some(parsed) => SourceParse::Analyzed(AnalyzedSource {
                path: path.to_owned(),
                parsed,
                script_ranges,
                role: SourceRole::ReadOnlyVue,
            }),
            None => SourceParse::Unsupported(limitation(
                path,
                "its TypeScript script block has syntax errors, so its imports cannot be counted safely",
            )),
        },
    })
}

enum VueExtract {
    NoTypeScript,
    Unsupported(AnalysisLimitation),
    TypeScript {
        embedded: String,
        dialect: TypeScriptDialect,
        script_ranges: Vec<(usize, usize)>,
    },
}

struct StartTag {
    name: String,
    attributes: Vec<(String, Option<String>)>,
    open_end: usize,
    self_closing: bool,
}

struct ScriptBlock {
    lang: ScriptLang,
    content_start: usize,
    content_end: usize,
}

enum ScriptLang {
    Ts,
    Tsx,
}

fn extract_vue_typescript(path: &str, source: &str) -> VueExtract {
    let bytes = source.as_bytes();
    let mut index = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let mut scripts = Vec::new();
    let mut saw_template = false;
    let mut saw_script = false;
    let mut saw_script_setup = false;
    while index < bytes.len() {
        index = skip_whitespace(bytes, index);
        if index >= bytes.len() {
            break;
        }
        if bytes[index] != b'<' {
            return VueExtract::Unsupported(limitation(
                path,
                "it has text outside a top-level block, so the component structure is ambiguous",
            ));
        }
        if source[index..].starts_with("<!--") {
            let Some(comment_end) = find_comment_end(source, index) else {
                return VueExtract::Unsupported(limitation(
                    path,
                    "it has an unclosed HTML comment",
                ));
            };
            index = comment_end;
            continue;
        }
        if source[index..].starts_with("<!") || source[index..].starts_with("<?") {
            return VueExtract::Unsupported(limitation(
                path,
                "it has an unsupported top-level markup declaration",
            ));
        }
        if source[index..].starts_with("</") {
            return VueExtract::Unsupported(limitation(path, "it has an unexpected closing tag"));
        }
        let Some(tag) = parse_start_tag(source, index) else {
            return VueExtract::Unsupported(limitation(
                path,
                "it has a top-level tag that could not be parsed",
            ));
        };
        if tag.self_closing {
            if tag.name == "script"
                && let Some(unsupported) =
                    note_script(path, &tag, &mut saw_script, &mut saw_script_setup)
            {
                return VueExtract::Unsupported(unsupported);
            }
            if tag.name == "template" {
                if saw_template {
                    return VueExtract::Unsupported(limitation(
                        path,
                        "it has more than one <template> block",
                    ));
                }
                saw_template = true;
            }
            index = tag.open_end;
            continue;
        }
        let close = if tag.name == "template" {
            if saw_template {
                return VueExtract::Unsupported(limitation(
                    path,
                    "it has more than one <template> block",
                ));
            }
            saw_template = true;
            find_nested_end(source, tag.open_end, "template")
        } else {
            find_raw_end(source, tag.open_end, &tag.name)
        };
        let Some((content_end, after)) = close else {
            return VueExtract::Unsupported(limitation(
                path,
                &format!("it has an unclosed <{}> block", tag.name),
            ));
        };
        if tag.name == "script" {
            if let Some(unsupported) =
                note_script(path, &tag, &mut saw_script, &mut saw_script_setup)
            {
                return VueExtract::Unsupported(unsupported);
            }
            if let Some(block) = supported_script(&tag, tag.open_end, content_end) {
                scripts.push(block);
            }
        }
        index = after;
    }
    if scripts.is_empty() {
        return VueExtract::NoTypeScript;
    }
    let dialect = if scripts
        .iter()
        .any(|script| matches!(script.lang, ScriptLang::Tsx))
    {
        TypeScriptDialect::Tsx
    } else {
        TypeScriptDialect::TypeScript
    };
    let Some(embedded) = embed_scripts(source, &scripts) else {
        return VueExtract::Unsupported(limitation(
            path,
            "its script block could not be embedded without splitting a character",
        ));
    };
    VueExtract::TypeScript {
        embedded,
        dialect,
        script_ranges: scripts
            .iter()
            .map(|script| (script.content_start, script.content_end))
            .collect(),
    }
}

fn note_script(
    path: &str,
    tag: &StartTag,
    saw_script: &mut bool,
    saw_script_setup: &mut bool,
) -> Option<AnalysisLimitation> {
    if let Some(limitation) = script_limitation(path, tag) {
        return Some(limitation);
    }
    let setup = tag.attributes.iter().any(|(name, _)| name == "setup");
    if setup {
        if *saw_script_setup {
            return Some(limitation(
                path,
                "it has more than one <script setup> block",
            ));
        }
        *saw_script_setup = true;
    } else if *saw_script {
        return Some(limitation(path, "it has more than one <script> block"));
    } else {
        *saw_script = true;
    }
    None
}

fn script_limitation(path: &str, tag: &StartTag) -> Option<AnalysisLimitation> {
    if tag.attributes.iter().any(|(name, _)| name == "src") {
        return Some(limitation(
            path,
            "it uses an external <script src> block, which Phase 1 does not inline",
        ));
    }
    let langs = tag
        .attributes
        .iter()
        .filter(|(name, _)| name == "lang")
        .count();
    if langs > 1 {
        return Some(limitation(
            path,
            "it has conflicting lang attributes on one <script> block",
        ));
    }
    let lang = tag
        .attributes
        .iter()
        .find_map(|(name, value)| (name == "lang").then_some(value.as_deref()).flatten());
    if matches!(lang, Some("ts" | "tsx")) {
        return None;
    }
    let detail = match lang {
        Some(value) => {
            format!("it has <script lang=\"{value}\">, which is not a supported TypeScript block")
        }
        None => "it has a <script> block without lang=\"ts\" or lang=\"tsx\"".to_owned(),
    };
    Some(limitation(path, &detail))
}

fn supported_script(
    tag: &StartTag,
    content_start: usize,
    content_end: usize,
) -> Option<ScriptBlock> {
    if tag.attributes.iter().any(|(name, _)| name == "src") {
        return None;
    }
    let lang = tag
        .attributes
        .iter()
        .find_map(|(name, value)| (name == "lang").then_some(value.as_deref()).flatten())?;
    let lang = match lang {
        "ts" => ScriptLang::Ts,
        "tsx" => ScriptLang::Tsx,
        _ => return None,
    };
    Some(ScriptBlock {
        lang,
        content_start,
        content_end,
    })
}

fn embed_scripts(source: &str, scripts: &[ScriptBlock]) -> Option<String> {
    let mut embedded = vec![b' '; source.len()];
    for script in scripts {
        if !source.is_char_boundary(script.content_start)
            || !source.is_char_boundary(script.content_end)
        {
            return None;
        }
        embedded[script.content_start..script.content_end]
            .copy_from_slice(&source.as_bytes()[script.content_start..script.content_end]);
    }
    String::from_utf8(embedded).ok()
}

fn limitation(path: &str, detail: &str) -> AnalysisLimitation {
    AnalysisLimitation {
        path: path.to_owned(),
        reason: ANALYSIS_LIMITATION_UNRESOLVED_VUE.to_owned(),
        message: format!(
            "{path} could not be analyzed: {detail}. type-tune did not treat this component as a non-referrer. Vue rewriting is not supported yet."
        ),
    }
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn find_comment_end(source: &str, start: usize) -> Option<usize> {
    source[start + 4..]
        .find("-->")
        .map(|relative| start + 4 + relative + 3)
}

fn find_raw_end(source: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let closing = format!("</{name}");
    let mut index = from;
    while index < bytes.len() {
        if bytes[index] == b'<'
            && source[index..].len() >= closing.len()
            && source[index..index + closing.len()].eq_ignore_ascii_case(&closing)
        {
            let mut cursor = index + closing.len();
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'>' {
                return Some((index, cursor + 1));
            }
        }
        index += 1;
    }
    None
}

fn find_nested_end(source: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut depth = 1i32;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if source[index..].starts_with("<!--") {
            index = find_comment_end(source, index)?;
            continue;
        }
        if source[index..].starts_with("</") {
            if let Some((tag_name, after)) = parse_end_tag(source, index) {
                if tag_name == name {
                    depth -= 1;
                    if depth == 0 {
                        return Some((index, after));
                    }
                }
                index = after;
                continue;
            }
            index += 1;
            continue;
        }
        if let Some(tag) = parse_start_tag(source, index) {
            if tag.name == name && !tag.self_closing {
                depth += 1;
            }
            index = tag.open_end;
            continue;
        }
        index += 1;
    }
    None
}

fn parse_end_tag(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if !source[start..].starts_with("</") {
        return None;
    }
    let mut index = start + 2;
    let name_start = index;
    if index >= bytes.len() || !is_name_start(bytes[index]) {
        return None;
    }
    index += 1;
    while index < bytes.len() && is_name_char(bytes[index]) {
        index += 1;
    }
    let name = source[name_start..index].to_ascii_lowercase();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'>' {
        Some((name, index + 1))
    } else {
        None
    }
}

fn parse_start_tag(source: &str, start: usize) -> Option<StartTag> {
    let bytes = source.as_bytes();
    if start >= bytes.len() || bytes[start] != b'<' || source[start..].starts_with("</") {
        return None;
    }
    let mut index = start + 1;
    let name_start = index;
    if index >= bytes.len() || !is_name_start(bytes[index]) {
        return None;
    }
    index += 1;
    while index < bytes.len() && is_name_char(bytes[index]) {
        index += 1;
    }
    let name = source[name_start..index].to_ascii_lowercase();
    let mut attributes = Vec::new();
    loop {
        index = skip_whitespace(bytes, index);
        if index >= bytes.len() {
            return None;
        }
        if source[index..].starts_with("/>") {
            return Some(StartTag {
                name,
                attributes,
                open_end: index + 2,
                self_closing: true,
            });
        }
        if bytes[index] == b'>' {
            return Some(StartTag {
                name,
                attributes,
                open_end: index + 1,
                self_closing: false,
            });
        }
        let attribute_start = index;
        while index < bytes.len() && is_attribute_char(bytes[index]) {
            index += 1;
        }
        if index == attribute_start {
            return None;
        }
        let attribute_name = source[attribute_start..index].to_ascii_lowercase();
        index = skip_whitespace(bytes, index);
        let value = if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            index = skip_whitespace(bytes, index);
            let (parsed, next) = parse_attribute_value(source, index)?;
            index = next;
            Some(parsed)
        } else {
            None
        };
        attributes.push((attribute_name, value));
    }
}

fn parse_attribute_value(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let quote = bytes[start];
    if quote == b'"' || quote == b'\'' {
        let mut index = start + 1;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let value = source[start + 1..index].to_ascii_lowercase();
        Some((value, index + 1))
    } else {
        let mut index = start;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
            index += 1;
        }
        if index == start {
            return None;
        }
        Some((source[start..index].to_ascii_lowercase(), index))
    }
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b':'
}

fn is_attribute_char(byte: u8) -> bool {
    !byte.is_ascii_whitespace() && !matches!(byte, b'"' | b'\'' | b'=' | b'>' | b'/')
}

#[cfg(test)]
mod tests {
    use super::parse_analysis_source;
    use crate::vue::SourceParse;

    fn script_of(source: &str) -> String {
        match parse_analysis_source("src/Card.vue", source).expect("parses") {
            SourceParse::Analyzed(analyzed) => analyzed
                .parsed
                .import_analysis
                .imports
                .iter()
                .map(|import| import.imported_name.clone())
                .collect::<Vec<_>>()
                .join(","),
            other => panic!(
                "expected analyzed script, got {}",
                unsupported_message(other)
            ),
        }
    }

    fn unsupported_message(parsed: SourceParse) -> String {
        match parsed {
            SourceParse::Unsupported(limitation) => limitation.message,
            SourceParse::NoTypeScript => "no typescript".into(),
            SourceParse::Analyzed(_) => "analyzed".into(),
        }
    }

    #[test]
    fn preserves_script_offsets_in_the_original_file() {
        let source = "<template><div>{{ User }}</div></template>\n<script setup lang=\"ts\">\nimport type { User } from \"./types\";\n</script>\n";
        let SourceParse::Analyzed(analyzed) =
            parse_analysis_source("src/Card.vue", source).unwrap()
        else {
            panic!("expected script");
        };
        let import = &analyzed.parsed.import_analysis.imports[0];
        let quoted = &source[import.module_specifier_start..import.module_specifier_end];
        assert_eq!(quoted, "\"./types\"");
        assert!(
            analyzed
                .parsed
                .import_analysis
                .usage
                .binding_names
                .is_empty()
        );
    }

    #[test]
    fn ignores_typescript_looking_template_and_style_text() {
        let source = r#"
<template>
import type { User } from "./types";
const user: User = null;
</template>
<style>
import type { User } from "./types";
</style>
<script setup lang="ts">
const title = "User";
</script>
"#;
        match parse_analysis_source("src/Card.vue", source).unwrap() {
            SourceParse::Analyzed(analyzed) => {
                assert!(analyzed.parsed.import_analysis.imports.is_empty());
                assert!(
                    !analyzed
                        .parsed
                        .import_analysis
                        .usage
                        .binding_names
                        .contains("User")
                );
            }
            other => panic!("{}", unsupported_message(other)),
        }
    }

    #[test]
    fn reports_malformed_and_unsupported_scripts() {
        for source in [
            "<script setup lang=\"ts\">\nimport type { User } from './types';\n",
            "<script>\nimport type { User } from './types';\n</script>\n",
            "<script setup lang=\"js\">\nexport default {}\n</script>\n",
            "<script src=\"./card.ts\" lang=\"ts\"></script>\n",
            "<script lang=\"ts\"></script><script lang=\"ts\"></script>\n",
        ] {
            let parsed = parse_analysis_source("src/Card.vue", source).unwrap();
            assert!(
                matches!(parsed, SourceParse::Unsupported(_)),
                "{}",
                unsupported_message(parsed)
            );
        }
    }

    #[test]
    fn shares_scope_between_script_and_script_setup() {
        let source = r#"
<script lang="ts">
import type { User } from "./types";
</script>
<script setup lang="ts">
defineProps<{ user: User }>();
</script>
"#;
        assert_eq!(script_of(source), "User");
        let SourceParse::Analyzed(analyzed) =
            parse_analysis_source("src/Card.vue", source).unwrap()
        else {
            panic!("expected script");
        };
        assert!(
            analyzed
                .parsed
                .import_analysis
                .usage
                .binding_names
                .contains("User")
        );
    }
}
