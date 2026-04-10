use std::path::Path;

use globset::Glob;
use tree_sitter::{Node, Parser, Point};

use crate::error::{Result, SmartEditError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstLanguage {
    Rust,
}

impl AstLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Some(Self::Rust),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AstRenderOptions {
    pub include_signatures: bool,
    pub include_type_bodies: bool,
    pub include_function_bodies: bool,
    pub include_docs: bool,
    pub include_locations: bool,
}

impl AstRenderOptions {
    pub fn basic() -> Self {
        Self::default()
    }
}

impl Default for AstRenderOptions {
    fn default() -> Self {
        Self {
            include_signatures: false,
            include_type_bodies: false,
            include_function_bodies: false,
            include_docs: false,
            include_locations: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAst {
    pub language: AstLanguage,
    pub root_docs: Option<String>,
    pub items: Vec<AstItem>,
    pub has_errors: bool,
}

impl FileAst {
    pub fn parse(language: AstLanguage, source: &str) -> Result<Self> {
        match language {
            AstLanguage::Rust => parse_rust_ast(source),
        }
    }

    pub fn render(&self, options: AstRenderOptions) -> String {
        self.render_items(&self.items, options)
    }

    pub fn render_with_selector(
        &self,
        selector: &AstSelector,
        options: AstRenderOptions,
    ) -> Result<String> {
        let items = self.select_items(selector)?;
        if items.is_empty() {
            return Err(SmartEditError::NoAstItemsMatched {
                selector: selector.display(),
            });
        }
        Ok(self.render_items(&items, options))
    }

    pub fn select_items(&self, selector: &AstSelector) -> Result<Vec<AstItem>> {
        let matchers = selector.compile()?;
        let mut selected = Vec::new();
        for item in &self.items {
            collect_selected_items(item, None, &matchers, &mut selected);
        }
        Ok(selected)
    }

    fn render_items(&self, items: &[AstItem], options: AstRenderOptions) -> String {
        let mut rendered = String::new();
        if options.include_docs {
            if let Some(root_docs) = self.root_docs.as_deref() {
                push_indented_block(&mut rendered, 0, root_docs, None);
            }
        }
        for (index, item) in items.iter().enumerate() {
            if !rendered.is_empty() || index > 0 {
                rendered.push('\n');
            }
            render_item(item, options, 0, &mut rendered);
        }
        rendered
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstItem {
    pub kind: AstItemKind,
    pub name: Option<String>,
    pub associated_type: Option<String>,
    pub location: AstLocationRange,
    pub docs: Option<String>,
    pub summary: String,
    pub signature: Option<String>,
    pub body: Option<String>,
    pub children: Vec<AstItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AstLocationRange {
    pub start_line: usize,
    pub end_line: usize,
}

impl AstLocationRange {
    pub fn from_points(start: Point, end: Point) -> Self {
        Self {
            start_line: start.row + 1,
            end_line: end.row + 1,
        }
    }

    pub fn display(self) -> String {
        format!("{}-{}", self.start_line, self.end_line)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstItemKind {
    Function,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Impl,
    Module,
    Const,
    Static,
}

impl AstItemKind {
    fn supports_type_bodies(self) -> bool {
        matches!(
            self,
            AstItemKind::Struct
                | AstItemKind::Enum
                | AstItemKind::Union
                | AstItemKind::TypeAlias
                | AstItemKind::Const
                | AstItemKind::Static
        )
    }

    fn supports_function_bodies(self) -> bool {
        matches!(self, AstItemKind::Function)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AstSelector {
    pub item_patterns: Vec<String>,
    pub type_patterns: Vec<String>,
}

impl AstSelector {
    pub fn is_empty(&self) -> bool {
        self.item_patterns.is_empty() && self.type_patterns.is_empty()
    }

    pub fn display(&self) -> String {
        let mut segments = Vec::new();
        segments.extend(
            self.item_patterns
                .iter()
                .map(|pattern| format!("-s {pattern}")),
        );
        segments.extend(
            self.type_patterns
                .iter()
                .map(|pattern| format!("-S {pattern}")),
        );
        segments.join(", ")
    }

    fn compile(&self) -> Result<CompiledAstSelector> {
        Ok(CompiledAstSelector {
            item_patterns: self
                .item_patterns
                .iter()
                .map(|pattern| compile_selector_pattern(pattern))
                .collect::<Result<Vec<_>>>()?,
            type_patterns: self
                .type_patterns
                .iter()
                .map(|pattern| compile_selector_pattern(pattern))
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

pub fn parse_file_ast(path: &Path, source: &str) -> Result<FileAst> {
    let language =
        AstLanguage::from_path(path).ok_or_else(|| SmartEditError::UnsupportedAstLanguage {
            path: path.to_path_buf(),
        })?;
    FileAst::parse(language, source)
}

fn parse_rust_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "rust",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "rust",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let docs = RustDocContext::new(source);

    Ok(FileAst {
        language: AstLanguage::Rust,
        root_docs: docs.root_module_docs(),
        items: collect_supported_items(root, &docs),
        has_errors: root.has_error(),
    })
}

fn collect_supported_items(node: Node<'_>, docs: &RustDocContext<'_>) -> Vec<AstItem> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter_map(|child| parse_item(child, docs))
        .collect()
}

fn parse_item(node: Node<'_>, docs: &RustDocContext<'_>) -> Option<AstItem> {
    match node.kind() {
        "function_item" | "function_signature_item" => Some(parse_function_item(node, docs)),
        "struct_item" => Some(parse_simple_item(AstItemKind::Struct, "struct", node, docs)),
        "enum_item" => Some(parse_simple_item(AstItemKind::Enum, "enum", node, docs)),
        "union_item" => Some(parse_simple_item(AstItemKind::Union, "union", node, docs)),
        "type_item" => Some(parse_simple_item(
            AstItemKind::TypeAlias,
            "type",
            node,
            docs,
        )),
        "const_item" => Some(parse_simple_item(AstItemKind::Const, "const", node, docs)),
        "static_item" => Some(parse_simple_item(AstItemKind::Static, "static", node, docs)),
        "trait_item" => Some(parse_trait_item(node, docs)),
        "impl_item" => Some(parse_impl_item(node, docs)),
        "mod_item" => Some(parse_mod_item(node, docs)),
        _ => None,
    }
}

fn parse_function_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node),
        docs: docs.leading_item_docs(node),
        summary: format!("fn {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children: Vec::new(),
    }
}

fn parse_simple_item(
    kind: AstItemKind,
    keyword: &str,
    node: Node<'_>,
    docs: &RustDocContext<'_>,
) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node),
        docs: docs.leading_item_docs(node),
        summary: format!("{keyword} {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children: Vec::new(),
    }
}

fn parse_trait_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let children = node
        .child_by_field_name("body")
        .map(|body| collect_supported_items(body, docs))
        .unwrap_or_default();
    AstItem {
        kind: AstItemKind::Trait,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node),
        docs: docs.leading_item_docs(node),
        summary: format!("trait {name}"),
        signature: Some(signature_text(node, source)),
        body: None,
        children,
    }
}

fn parse_impl_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let target =
        child_text_by_field(node, "type", source).unwrap_or_else(|| "<unknown>".to_owned());
    let associated_type = extract_type_name(&target);
    let summary = if let Some(trait_name) = child_text_by_field(node, "trait", source) {
        format!("impl {trait_name} for {target}")
    } else {
        format!("impl {target}")
    };
    let children = node
        .child_by_field_name("body")
        .map(|body| collect_supported_items(body, docs))
        .unwrap_or_default();
    AstItem {
        kind: AstItemKind::Impl,
        name: None,
        associated_type,
        location: location_for_node(node),
        docs: docs.leading_item_docs(node),
        summary,
        signature: Some(signature_text(node, source)),
        body: None,
        children,
    }
}

fn parse_mod_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let children = node
        .child_by_field_name("body")
        .map(|body| collect_supported_items(body, docs))
        .unwrap_or_default();
    AstItem {
        kind: AstItemKind::Module,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node),
        docs: docs.leading_item_docs(node),
        summary: format!("mod {name}"),
        signature: Some(signature_text(node, source)),
        body: None,
        children,
    }
}

struct RustDocContext<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> RustDocContext<'a> {
    fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    fn root_module_docs(&self) -> Option<String> {
        let mut row = 0;
        let mut start_row = None;
        let mut end_row = None;
        while row < self.line_starts.len() {
            let line = self.line_text(row).trim();
            if line.is_empty() {
                if end_row.is_some() {
                    break;
                }
                row += 1;
                continue;
            }
            if is_rust_inner_doc_line_comment(line) {
                start_row.get_or_insert(row);
                end_row = Some(row);
                row += 1;
                continue;
            }
            if line.starts_with("#![") {
                if end_row.is_some() {
                    break;
                }
                row += 1;
                continue;
            }
            if is_doc_block_start(line) {
                if !line.starts_with("/*!") {
                    break;
                }
                start_row.get_or_insert(row);
                let end = self.scan_doc_block_end(row)?;
                end_row = Some(end);
                row = end + 1;
                continue;
            }
            break;
        }
        start_row
            .zip(end_row)
            .map(|(start_row, end_row)| self.rows_text(start_row, end_row))
    }

    fn leading_item_docs(&self, node: Node<'_>) -> Option<String> {
        let start_row = node.start_position().row;
        if start_row == 0 {
            return None;
        }

        let end_row = start_row - 1;
        let mut row = end_row;
        let mut start_row = None;

        loop {
            let line = self.line_text(row).trim();
            if line.is_empty() {
                break;
            }
            if is_rust_outer_doc_line_comment(line) {
                start_row = Some(row);
            } else if is_doc_block_end_candidate(line) {
                let block_start = self.scan_outer_doc_block_start(row)?;
                start_row = Some(block_start);
                row = block_start;
            } else {
                break;
            }

            if row == 0 {
                break;
            }
            row -= 1;
        }

        start_row.map(|start_row| self.rows_text(start_row, end_row))
    }

    fn scan_outer_doc_block_start(&self, mut row: usize) -> Option<usize> {
        loop {
            let line = self.line_text(row).trim();
            if line.starts_with("/**") {
                return Some(row);
            }
            if line.starts_with("/*") {
                return None;
            }
            if row == 0 {
                return None;
            }
            row -= 1;
        }
    }

    fn scan_doc_block_end(&self, mut row: usize) -> Option<usize> {
        loop {
            let line = self.line_text(row).trim();
            if line.ends_with("*/") {
                return Some(row);
            }
            row += 1;
            if row >= self.line_starts.len() {
                return None;
            }
        }
    }

    fn rows_text(&self, start_row: usize, end_row: usize) -> String {
        let start = self.line_start(start_row);
        let end = self.line_end(end_row);
        self.source[start..end].trim().to_owned()
    }

    fn line_text(&self, row: usize) -> &'a str {
        let start = self.line_start(row);
        let end = self.line_end(row);
        &self.source[start..end]
    }

    fn line_start(&self, row: usize) -> usize {
        self.line_starts[row]
    }

    fn line_end(&self, row: usize) -> usize {
        let mut end = self
            .line_starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.source.len());
        if end > 0 && self.source.as_bytes()[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && self.source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end
    }
}

fn is_rust_outer_doc_line_comment(line: &str) -> bool {
    line.starts_with("///") && !line.starts_with("////")
}

fn is_rust_inner_doc_line_comment(line: &str) -> bool {
    line.starts_with("//!")
}

fn is_doc_block_start(line: &str) -> bool {
    line.starts_with("/**") || line.starts_with("/*!")
}

fn is_doc_block_end_candidate(line: &str) -> bool {
    line.ends_with("*/")
}

fn signature_text(node: Node<'_>, source: &str) -> String {
    if let Some(body) = node.child_by_field_name("body") {
        return source_fragment(source, node.start_byte(), body.start_byte());
    }
    trimmed_node_text(node, source)
}

fn child_text_by_field(node: Node<'_>, field_name: &str, source: &str) -> Option<String> {
    node.child_by_field_name(field_name)
        .map(|child| trimmed_text(source, child.start_byte(), child.end_byte()))
}

fn trimmed_node_text(node: Node<'_>, source: &str) -> String {
    trimmed_text(source, node.start_byte(), node.end_byte())
}

fn source_fragment(source: &str, start: usize, end: usize) -> String {
    trimmed_text(source, start, end)
}

fn trimmed_text(source: &str, start: usize, end: usize) -> String {
    source[start..end].trim().to_owned()
}

fn location_for_node(node: Node<'_>) -> AstLocationRange {
    AstLocationRange::from_points(node.start_position(), node.end_position())
}

fn extract_type_name(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_generics = trimmed.split('<').next().unwrap_or(trimmed).trim();
    let without_prefix = without_generics
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let last_segment = without_prefix
        .rsplit("::")
        .next()
        .unwrap_or(without_prefix)
        .trim();
    let candidate = last_segment
        .strip_prefix("dyn ")
        .unwrap_or(last_segment)
        .strip_prefix("impl ")
        .unwrap_or(last_segment)
        .trim();

    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_owned())
    }
}

#[derive(Debug)]
struct CompiledAstSelector {
    item_patterns: Vec<globset::GlobMatcher>,
    type_patterns: Vec<globset::GlobMatcher>,
}

fn compile_selector_pattern(pattern: &str) -> Result<globset::GlobMatcher> {
    Glob::new(pattern)
        .map(|glob| glob.compile_matcher())
        .map_err(|error| SmartEditError::InvalidAstSelectorPattern {
            pattern: pattern.to_owned(),
            message: error.to_string(),
        })
}

fn collect_selected_items(
    item: &AstItem,
    parent_context: Option<&str>,
    selector: &CompiledAstSelector,
    selected: &mut Vec<AstItem>,
) {
    let item_path = selector_path_for_item(item, parent_context);
    if item_matches_selector(item, item_path.as_deref(), selector) {
        selected.push(item.clone());
        return;
    }

    let child_context = child_context_for_item(item, parent_context);
    for child in &item.children {
        collect_selected_items(child, child_context.as_deref(), selector, selected);
    }
}

fn item_matches_selector(
    item: &AstItem,
    item_path: Option<&str>,
    selector: &CompiledAstSelector,
) -> bool {
    let item_match = item_path
        .map(|path| {
            selector
                .item_patterns
                .iter()
                .any(|pattern| pattern.is_match(path))
        })
        .unwrap_or(false);
    let type_match = item_supports_type_selection(item)
        && item
            .associated_type
            .as_deref()
            .or(item.name.as_deref())
            .map(|name| {
                selector
                    .type_patterns
                    .iter()
                    .any(|pattern| pattern.is_match(name))
            })
            .unwrap_or(false);
    item_match || type_match
}

fn item_supports_type_selection(item: &AstItem) -> bool {
    matches!(
        item.kind,
        AstItemKind::Struct
            | AstItemKind::Enum
            | AstItemKind::Union
            | AstItemKind::TypeAlias
            | AstItemKind::Trait
            | AstItemKind::Impl
    )
}

fn selector_path_for_item(item: &AstItem, parent_context: Option<&str>) -> Option<String> {
    match item.kind {
        AstItemKind::Impl => item
            .associated_type
            .as_deref()
            .map(|name| join_selector_path(parent_context, name)),
        _ => item
            .name
            .as_deref()
            .map(|name| join_selector_path(parent_context, name)),
    }
}

fn child_context_for_item(item: &AstItem, parent_context: Option<&str>) -> Option<String> {
    match item.kind {
        AstItemKind::Module => item
            .name
            .as_deref()
            .map(|name| join_selector_path(parent_context, name)),
        AstItemKind::Impl => item
            .associated_type
            .as_deref()
            .map(|name| join_selector_path(parent_context, name)),
        _ => selector_path_for_item(item, parent_context),
    }
}

fn join_selector_path(parent: Option<&str>, segment: &str) -> String {
    match parent {
        Some(parent) if !parent.is_empty() => format!("{parent}.{segment}"),
        _ => segment.to_owned(),
    }
}

fn render_item(item: &AstItem, options: AstRenderOptions, indent: usize, output: &mut String) {
    let render_full_body = item.kind.supports_function_bodies() && options.include_function_bodies
        || item.kind.supports_type_bodies() && options.include_type_bodies;
    let text = if render_full_body {
        item.body.as_deref().unwrap_or(&item.summary)
    } else if options.include_signatures {
        item.signature.as_deref().unwrap_or(&item.summary)
    } else {
        &item.summary
    };

    if options.include_docs {
        if let Some(docs) = item.docs.as_deref() {
            push_indented_block(output, indent, docs, None);
            output.push('\n');
        }
    }

    push_indented_block(
        output,
        indent,
        text,
        options.include_locations.then(|| item.location.display()),
    );

    if !render_full_body {
        for child in &item.children {
            output.push('\n');
            render_item(child, options, indent + 1, output);
        }
    }
}

fn push_indented_block(output: &mut String, indent: usize, text: &str, location: Option<String>) {
    let prefix = indent_prefix(indent);
    let normalized = normalize_indentation(text);
    let location_prefix = location.map(|location| format!("[{location}] "));
    let continuation_prefix = match &location_prefix {
        Some(location_prefix) => format!("{prefix}{}", " ".repeat(location_prefix.len())),
        None => prefix.clone(),
    };
    for (index, line) in normalized.lines().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        if !line.is_empty() {
            if index == 0 {
                output.push_str(&prefix);
                if let Some(location_prefix) = &location_prefix {
                    output.push_str(location_prefix);
                }
            } else {
                output.push_str(&continuation_prefix);
            }
            output.push_str(line);
        }
    }
    if text.is_empty() {
        output.push_str(&prefix);
    }
}

fn indent_prefix(indent: usize) -> String {
    match indent {
        0 => String::new(),
        _ => format!("{} ", ">".repeat(indent)),
    }
}

fn normalize_indentation(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 1 {
        return text.to_owned();
    }

    let shared_indent = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
        })
        .min()
        .unwrap_or(0);

    let mut normalized = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            normalized.push('\n');
        }
        if index == 0 || line.trim().is_empty() {
            normalized.push_str(line);
        } else {
            normalized.push_str(&line.chars().skip(shared_indent).collect::<String>());
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{AstLanguage, AstRenderOptions, AstSelector, FileAst};

    const SAMPLE: &str = r#"
struct S {
    a: bool,
}

enum E {
    A,
    B,
}

fn f1(a: String) -> bool {
    !a.is_empty()
}

impl S {
    fn f2(&self) -> bool {
        self.a
    }
}
"#;

    #[test]
    fn renders_basic_rust_outline() {
        let ast = FileAst::parse(AstLanguage::Rust, SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "struct S\nenum E\nfn f1\nimpl S\n> fn f2"
        );
    }

    #[test]
    fn renders_signatures_when_requested() {
        let ast = FileAst::parse(AstLanguage::Rust, SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            }),
            "struct S\nenum E\nfn f1(a: String) -> bool\nimpl S\n> fn f2(&self) -> bool"
        );
    }

    #[test]
    fn renders_type_and_function_bodies_when_requested() {
        let ast = FileAst::parse(AstLanguage::Rust, SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_type_bodies: true,
                include_function_bodies: true,
                include_docs: false,
                include_locations: false,
            }),
            "struct S {\n    a: bool,\n}\nenum E {\n    A,\n    B,\n}\nfn f1(a: String) -> bool {\n    !a.is_empty()\n}\nimpl S\n> fn f2(&self) -> bool {\n>     self.a\n> }"
        );
    }

    #[test]
    fn selects_module_items_by_glob_path() {
        let ast = FileAst::parse(AstLanguage::Rust, "mod xyz { fn inner() {} struct S; }").unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["xyz.*".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "fn inner\nstruct S");
    }

    #[test]
    fn selects_top_level_items_by_name_without_file_prefix() {
        let ast = FileAst::parse(AstLanguage::Rust, "fn f1() {} fn f2() {}").unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["f1".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "fn f1");
    }

    #[test]
    fn selects_type_and_associated_impls() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "struct S1; impl S1 { fn new() -> Self { Self } } fn other() {}",
        )
        .unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["S1".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "struct S1\nimpl S1\n> fn new");
    }

    #[test]
    fn renders_locations_when_requested() {
        let ast = FileAst::parse(AstLanguage::Rust, SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_locations: true,
                ..AstRenderOptions::default()
            }),
            "[2-4] struct S\n[6-9] enum E\n[11-13] fn f1\n[15-19] impl S\n> [16-18] fn f2"
        );
    }

    #[test]
    fn renders_deeper_nesting_with_repeated_markers() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "mod outer { mod inner { fn deep() {} } }",
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "mod outer\n> mod inner\n>> fn deep"
        );
    }

    #[test]
    fn renders_docs_when_requested() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "//! crate docs\n//! second line\n\n/// struct docs\nstruct S;\n\n/// module docs\nmod inner {\n    /// nested docs\n    fn deep() {}\n}\n",
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "//! crate docs\n//! second line\n/// struct docs\nstruct S\n/// module docs\nmod inner\n> /// nested docs\n> fn deep"
        );
    }

    #[test]
    fn renders_docs_across_attrs_and_crate_attributes() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "#![allow(dead_code)]\n//! crate docs\n\n/// struct docs\n#[derive(Debug)]\nstruct S;\n",
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "//! crate docs\n/// struct docs\nstruct S"
        );
    }
}
