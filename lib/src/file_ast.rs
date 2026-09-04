use std::path::Path;

use globset::Glob;
use tree_sitter::{Node, Parser, Point};

use crate::error::{Result, SmartEditError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Scala,
    Java,
    Kotlin,
    Lua,
}

impl AstLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Some(Self::Rust),
            Some("py" | "pyi") => Some(Self::Python),
            Some("js") | Some("mjs") | Some("cjs") | Some("jsx") => Some(Self::JavaScript),
            Some("ts") | Some("mts") | Some("cts") => Some(Self::TypeScript),
            Some("tsx") => Some(Self::Tsx),
            Some("go") => Some(Self::Go),
            Some("scala" | "sc") => Some(Self::Scala),
            Some("java") => Some(Self::Java),
            Some("kt" | "kts") => Some(Self::Kotlin),
            Some("lua") => Some(Self::Lua),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAst {
    pub language: AstLanguage,
    pub root_docs: Option<String>,
    pub items: Vec<AstItem>,
    pub has_errors: bool,
    pub first_error: Option<AstSyntaxErrorLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Location of the first syntax recovery node reported by Tree-sitter.
///
/// Both coordinates are zero-based. `column` is a byte column, matching Tree-sitter's
/// [`Point`] representation rather than a Unicode display column.
pub struct AstSyntaxErrorLocation {
    /// Zero-based source line.
    pub line: usize,
    /// Zero-based byte column within `line`.
    pub column: usize,
    /// Whether the parser reported a missing token rather than an explicit error node.
    pub is_missing: bool,
}

impl FileAst {
    pub fn parse(language: AstLanguage, source: &str) -> Result<Self> {
        match language {
            AstLanguage::Rust => parse_rust_ast(source),
            AstLanguage::Python => parse_python_ast(source),
            AstLanguage::JavaScript | AstLanguage::TypeScript | AstLanguage::Tsx => {
                parse_js_like_ast(language, source)
            }
            AstLanguage::Go => parse_go_ast(source),
            AstLanguage::Scala => parse_scala_ast(source),
            AstLanguage::Java => parse_java_ast(source),
            AstLanguage::Kotlin => parse_kotlin_ast(source),
            AstLanguage::Lua => parse_lua_ast(source),
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
        if options.include_docs
            && let Some(root_docs) = self.root_docs.as_deref()
        {
            push_indented_block(&mut rendered, 0, root_docs, None, false);
        }
        for (index, item) in items.iter().enumerate() {
            if !rendered.is_empty() || index > 0 {
                rendered.push('\n');
            }
            render_item(item, self.language, options, 0, &mut rendered);
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
    pub inner_docs: Option<String>,
    pub attributes: Option<String>,
    pub source_preamble: Option<String>,
    pub summary: String,
    pub signature: Option<String>,
    pub body: Option<String>,
    pub children: Vec<AstItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A zero-based, half-open whole-line source range.
pub struct AstLocationRange {
    /// First included line.
    pub start_line: usize,
    /// First line after the item.
    pub end_line: usize,
    /// Whether no unrelated source shares either boundary line.
    pub is_edit_ready: bool,
}

impl AstLocationRange {
    /// Converts Tree-sitter points to a line range without claiming whole-line ownership.
    pub fn from_points(start: Point, end: Point) -> Self {
        Self {
            start_line: start.row,
            end_line: end.row + usize::from(end.column > 0),
            // Points alone cannot prove that no other source shares either line.
            is_edit_ready: false,
        }
    }

    /// Formats the range, appending `shared-line` when it is not safe for a whole-line edit.
    pub fn display(self) -> String {
        let annotation = if self.is_edit_ready {
            ""
        } else {
            " shared-line"
        };
        format!("{}-{}{}", self.start_line, self.end_line, annotation)
    }

    fn from_source_span(
        source: &str,
        start: Point,
        end: Point,
        start_byte: usize,
        end_byte: usize,
    ) -> Self {
        let mut location = Self::from_points(start, end);
        let line_start = source[..start_byte]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let starts_on_owned_line = source[line_start..start_byte].trim().is_empty();
        let ends_on_owned_line = end.column == 0
            || source[end_byte..]
                .split_once('\n')
                .map_or(&source[end_byte..], |(line, _)| line)
                .trim()
                .is_empty();
        location.is_edit_ready = starts_on_owned_line && ends_on_owned_line;
        location
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstItemKind {
    Function,
    Class,
    Interface,
    Struct,
    Enum,
    Union,
    TypeAlias,
    Trait,
    Impl,
    Module,
    Const,
    Static,
    Use,
    Macro,
    MacroInvocation,
    ForeignBlock,
    Field,
}

impl AstItemKind {
    fn supports_type_bodies(self) -> bool {
        matches!(
            self,
            AstItemKind::Class
                | AstItemKind::Interface
                | AstItemKind::Struct
                | AstItemKind::Enum
                | AstItemKind::Union
                | AstItemKind::TypeAlias
                | AstItemKind::Trait
                | AstItemKind::Impl
                | AstItemKind::Module
                | AstItemKind::Const
                | AstItemKind::Static
                | AstItemKind::Macro
                | AstItemKind::ForeignBlock
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
                .map(|pattern| {
                    Ok(CompiledItemSelectorPattern {
                        matcher: compile_selector_pattern(pattern)?,
                        is_qualified: pattern.contains('.'),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            type_patterns: self
                .type_patterns
                .iter()
                .map(|pattern| {
                    Ok(CompiledTypeSelectorPattern {
                        matcher: compile_selector_pattern(pattern)?,
                        is_qualified: pattern.contains('.'),
                    })
                })
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
    let mut items = collect_supported_items(root, &docs);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Rust,
        root_docs: docs.inner_docs(root),
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_python_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "python",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "python",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let docs = PythonDocContext { source };
    let mut items = collect_python_supported_items(root, &docs);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Python,
        root_docs: docs.root_module_docs(root),
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_go_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "go",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "go",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let docs = GoDocContext::new(source, root);
    let mut items = collect_go_supported_items(root, &docs);
    mark_overlapping_sibling_locations(&mut items);
    let syntax_error = first_syntax_error(root);
    let shape_error = first_go_source_shape_error(root, source);

    Ok(FileAst {
        language: AstLanguage::Go,
        root_docs: docs.root_module_docs(),
        items,
        has_errors: root.has_error() || shape_error.is_some(),
        first_error: earliest_syntax_error(syntax_error, shape_error),
    })
}

fn first_go_source_shape_error(root: Node<'_>, source: &str) -> Option<AstSyntaxErrorLocation> {
    let mut cursor = root.walk();
    let syntax = root
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .collect::<Vec<_>>();
    let Some(first) = syntax.first().copied() else {
        let point = point_for_byte(source, source.len());
        return Some(AstSyntaxErrorLocation {
            line: point.row,
            column: point.column,
            is_missing: true,
        });
    };
    if first.kind() != "package_clause" {
        return Some(go_shape_error_at(first, true));
    }

    for child in syntax.into_iter().skip(1) {
        if child.kind() == "package_clause"
            || !matches!(
                child.kind(),
                "import_declaration"
                    | "const_declaration"
                    | "var_declaration"
                    | "type_declaration"
                    | "function_declaration"
                    | "method_declaration"
            )
        {
            return Some(go_shape_error_at(child, false));
        }
    }
    None
}

fn go_shape_error_at(node: Node<'_>, is_missing: bool) -> AstSyntaxErrorLocation {
    let point = node.start_position();
    AstSyntaxErrorLocation {
        line: point.row,
        column: point.column,
        is_missing,
    }
}

fn earliest_syntax_error(
    first: Option<AstSyntaxErrorLocation>,
    second: Option<AstSyntaxErrorLocation>,
) -> Option<AstSyntaxErrorLocation> {
    match (first, second) {
        (Some(first), Some(second)) => Some(
            if (first.line, first.column) <= (second.line, second.column) {
                first
            } else {
                second
            },
        ),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn parse_js_like_ast(language: AstLanguage, source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let (tree_sitter_language, language_name, flavor) = match language {
        AstLanguage::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            JsLikeFlavor::JavaScript,
        ),
        AstLanguage::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            JsLikeFlavor::TypeScript,
        ),
        AstLanguage::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "tsx",
            JsLikeFlavor::TypeScript,
        ),
        AstLanguage::Rust
        | AstLanguage::Python
        | AstLanguage::Go
        | AstLanguage::Scala
        | AstLanguage::Java
        | AstLanguage::Kotlin
        | AstLanguage::Lua => unreachable!(),
    };
    parser
        .set_language(&tree_sitter_language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: language_name,
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: language_name,
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let docs = JsDocContext::new(source, root);
    let mut items = collect_js_like_supported_items(root, &docs, flavor);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language,
        root_docs: docs.root_module_docs(),
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn first_syntax_error(root: Node<'_>) -> Option<AstSyntaxErrorLocation> {
    fn visit(node: Node<'_>) -> Option<Node<'_>> {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        let mut cursor = node.walk();
        node.children(&mut cursor).find_map(visit)
    }

    visit(root).map(|node| {
        let point = node.start_position();
        AstSyntaxErrorLocation {
            line: point.row,
            column: point.column,
            is_missing: node.is_missing(),
        }
    })
}

fn mark_overlapping_sibling_locations(items: &mut [AstItem]) {
    for index in 0..items.len() {
        for other in index + 1..items.len() {
            let overlaps = items[index].location.start_line < items[other].location.end_line
                && items[other].location.start_line < items[index].location.end_line;
            if overlaps {
                items[index].location.is_edit_ready = false;
                items[other].location.is_edit_ready = false;
            }
        }
    }
    for item in items {
        mark_overlapping_sibling_locations(&mut item.children);
    }
}

fn collect_supported_items(node: Node<'_>, docs: &RustDocContext<'_>) -> Vec<AstItem> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter_map(|child| parse_item(child, docs))
        .collect()
}

fn collect_python_supported_items(node: Node<'_>, docs: &PythonDocContext<'_>) -> Vec<AstItem> {
    let mut cursor = node.walk();
    let mut items = Vec::new();
    for child in node.named_children(&mut cursor) {
        if let Some(item) = parse_python_item(child, docs) {
            items.push(item);
        } else {
            items.extend(collect_python_supported_items(child, docs));
        }
    }
    items
}

fn collect_go_supported_items(node: Node<'_>, docs: &GoDocContext<'_>) -> Vec<AstItem> {
    let container = if node.kind() == "block" {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| child.kind() == "statement_list")
            .unwrap_or(node)
    } else {
        node
    };
    let mut cursor = node.walk();
    container
        .named_children(&mut cursor)
        .flat_map(|child| parse_go_items(child, docs))
        .collect()
}

fn collect_js_like_supported_items(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let mut cursor = node.walk();
    let mut items = Vec::new();
    for child in node.named_children(&mut cursor) {
        items.extend(parse_js_like_items(child, docs, flavor));
        if flavor == JsLikeFlavor::TypeScript {
            items.extend(parse_ts_constructor_parameter_properties(child, docs));
        }
    }
    items
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
        "associated_type" => Some(parse_simple_item(
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
        "use_declaration" => Some(parse_use_item(node, docs)),
        "macro_definition" => Some(parse_macro_item(node, docs)),
        "macro_invocation" => Some(parse_macro_invocation(node, node, docs)),
        "expression_statement" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| child.kind() == "macro_invocation")
                .map(|invocation| parse_macro_invocation(node, invocation, docs))
        }
        "foreign_mod_item" => Some(parse_foreign_mod_item(node, docs)),
        _ => None,
    }
}

fn parse_python_item(node: Node<'_>, docs: &PythonDocContext<'_>) -> Option<AstItem> {
    match node.kind() {
        "function_definition" => Some(parse_python_function_item(node, node, docs)),
        "class_definition" => Some(parse_python_class_item(node, node, docs)),
        "decorated_definition" => parse_python_decorated_item(node, docs),
        "type_alias_statement" => Some(parse_python_type_alias_item(node, docs)),
        _ => None,
    }
}

fn parse_python_decorated_item(node: Node<'_>, docs: &PythonDocContext<'_>) -> Option<AstItem> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => return Some(parse_python_function_item(node, child, docs)),
            "class_definition" => return Some(parse_python_class_item(node, child, docs)),
            _ => {}
        }
    }
    None
}

fn parse_function_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("fn {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children: node
            .child_by_field_name("body")
            .map(|body| {
                collect_supported_items(body, docs)
                    .into_iter()
                    .filter(|item| item.kind != AstItemKind::MacroInvocation)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn parse_python_function_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &PythonDocContext<'_>,
) -> AstItem {
    let source = docs.source;
    let name = child_text_by_field(definition_node, "name", source)
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let body = definition_node.child_by_field_name("body");
    let is_async = signature_text(definition_node, source).starts_with("async def ");
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: body.and_then(|body| docs.docstring_for_body(body)),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: if is_async {
            format!("async def {name}")
        } else {
            format!("def {name}")
        },
        signature: Some(python_signature_text(render_node, definition_node, source)),
        body: Some(python_source_text(render_node, source)),
        children: body
            .map(|body| collect_python_supported_items(body, docs))
            .unwrap_or_default(),
    }
}

fn parse_python_class_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &PythonDocContext<'_>,
) -> AstItem {
    let source = docs.source;
    let name = child_text_by_field(definition_node, "name", source)
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let body = definition_node.child_by_field_name("body");
    AstItem {
        kind: AstItemKind::Class,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: body.and_then(|body| docs.docstring_for_body(body)),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("class {name}"),
        signature: Some(python_signature_text(render_node, definition_node, source)),
        body: Some(python_source_text(render_node, source)),
        children: body
            .map(|body| collect_python_supported_items(body, docs))
            .unwrap_or_default(),
    }
}

fn parse_python_type_alias_item(node: Node<'_>, docs: &PythonDocContext<'_>) -> AstItem {
    let source = docs.source;
    let name = node
        .child_by_field_name("left")
        .and_then(python_type_alias_name)
        .map(|name| trimmed_node_text(name, source))
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let text = python_source_text(node, source);
    AstItem {
        kind: AstItemKind::TypeAlias,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: None,
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("type {name}"),
        signature: Some(text.clone()),
        body: Some(text),
        children: Vec::new(),
    }
}

fn python_type_alias_name(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(python_type_alias_name)
}

fn parse_go_items(node: Node<'_>, docs: &GoDocContext<'_>) -> Vec<AstItem> {
    match node.kind() {
        "function_declaration" => vec![parse_go_function_item(node, docs, false)],
        "method_declaration" => vec![parse_go_function_item(node, docs, true)],
        "type_declaration" => parse_go_type_declaration_items(node, docs),
        "const_declaration" => {
            parse_go_value_declaration_items(AstItemKind::Const, "const", node, docs)
        }
        "var_declaration" => {
            parse_go_value_declaration_items(AstItemKind::Static, "var", node, docs)
        }
        _ => Vec::new(),
    }
}

fn parse_go_function_item(node: Node<'_>, docs: &GoDocContext<'_>, is_method: bool) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let receiver = node
        .child_by_field_name("receiver")
        .and_then(|receiver| extract_go_receiver_type(receiver, source));
    let summary = if is_method {
        match receiver.as_deref() {
            Some(receiver) => format!("method {receiver}.{name}"),
            None => format!("method {name}"),
        }
    } else {
        format!("func {name}")
    };
    let preamble = docs.leading_item_preamble(node);
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name),
        associated_type: receiver,
        location: location_for_go_node(node, preamble.as_ref(), source),
        docs: preamble.as_ref().map(|preamble| preamble.text.clone()),
        inner_docs: None,
        attributes: preamble
            .as_ref()
            .and_then(|preamble| preamble.directives.clone()),
        source_preamble: preamble.as_ref().map(|preamble| preamble.text.clone()),
        summary,
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children: node
            .child_by_field_name("body")
            .map(|body| collect_go_supported_items(body, docs))
            .unwrap_or_default(),
    }
}

fn parse_go_type_declaration_items(node: Node<'_>, docs: &GoDocContext<'_>) -> Vec<AstItem> {
    let grouped = go_declaration_is_grouped(node);
    let group_preamble = grouped.then(|| docs.leading_item_preamble(node)).flatten();
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| matches!(child.kind(), "type_spec" | "type_alias"))
        .enumerate()
        .map(|(index, spec)| {
            parse_go_type_spec_item(
                node,
                spec,
                grouped,
                (index == 0).then_some(group_preamble.as_ref()).flatten(),
                docs,
            )
        })
        .collect()
}

fn parse_go_type_spec_item(
    render_node: Node<'_>,
    spec: Node<'_>,
    grouped: bool,
    group_preamble: Option<&GoItemPreamble>,
    docs: &GoDocContext<'_>,
) -> AstItem {
    let source = docs.source;
    let name =
        child_text_by_field(spec, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let type_node = spec.child_by_field_name("type");
    let kind = match type_node.map(|node| node.kind()) {
        Some("struct_type") => AstItemKind::Struct,
        Some("interface_type") => AstItemKind::Interface,
        _ => AstItemKind::TypeAlias,
    };
    let keyword = match kind {
        AstItemKind::Struct => "struct",
        AstItemKind::Interface => "interface",
        _ => "type",
    };
    let owned_node = if grouped { spec } else { render_node };
    let owned_preamble = docs.leading_item_preamble(owned_node);
    let preamble = merge_go_preambles(group_preamble, owned_preamble.as_ref());
    let declaration = go_prefixed_spec("type", owned_node, spec, source);
    let signature = type_node
        .filter(|node| matches!(node.kind(), "struct_type" | "interface_type"))
        .map(|type_node| {
            let prefix = source_fragment(source, spec.start_byte(), type_node.start_byte());
            format!(
                "type {prefix} {}",
                type_node.kind().trim_end_matches("_type")
            )
        })
        .unwrap_or_else(|| declaration.clone());
    AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_go_node(owned_node, owned_preamble.as_ref(), source),
        docs: preamble.as_ref().map(|preamble| preamble.text.clone()),
        inner_docs: None,
        attributes: preamble
            .as_ref()
            .and_then(|preamble| preamble.directives.clone()),
        source_preamble: preamble.as_ref().map(|preamble| preamble.text.clone()),
        summary: format!("{keyword} {name}"),
        signature: Some(signature),
        body: Some(declaration),
        children: type_node
            .map(|type_node| parse_go_type_members(type_node, docs))
            .unwrap_or_default(),
    }
}

fn parse_go_value_declaration_items(
    kind: AstItemKind,
    keyword: &str,
    node: Node<'_>,
    docs: &GoDocContext<'_>,
) -> Vec<AstItem> {
    let source = docs.source;
    let grouped = go_declaration_is_grouped(node);
    let group_preamble = grouped.then(|| docs.leading_item_preamble(node)).flatten();
    let mut items = Vec::new();
    for (spec_index, spec) in go_declaration_specs(node, matches!(kind, AstItemKind::Const))
        .into_iter()
        .enumerate()
    {
        let owned_node = if grouped { spec } else { node };
        let owned_preamble = docs.leading_item_preamble(owned_node);
        let declaration = go_prefixed_spec(keyword, owned_node, spec, source);
        for (name_index, name) in go_spec_names(spec, source).into_iter().enumerate() {
            let context = (spec_index == 0 && name_index == 0)
                .then_some(group_preamble.as_ref())
                .flatten();
            let preamble = merge_go_preambles(context, owned_preamble.as_ref());
            items.push(AstItem {
                kind,
                name: Some(name.clone()),
                associated_type: None,
                location: location_for_go_node(owned_node, owned_preamble.as_ref(), source),
                docs: preamble.as_ref().map(|preamble| preamble.text.clone()),
                inner_docs: None,
                attributes: preamble
                    .as_ref()
                    .and_then(|preamble| preamble.directives.clone()),
                source_preamble: preamble.as_ref().map(|preamble| preamble.text.clone()),
                summary: format!("{keyword} {name}"),
                signature: Some(declaration.clone()),
                body: Some(declaration.clone()),
                children: Vec::new(),
            });
        }
    }
    items
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsLikeFlavor {
    JavaScript,
    TypeScript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsLikeFunctionKind {
    Function,
    Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct JsLikeFunctionModifiers {
    is_async: bool,
    is_abstract: bool,
    is_generator: bool,
    is_static: bool,
}

fn parse_js_like_items(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    match node.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            vec![parse_js_class_item(node, node, docs, flavor, None)]
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            vec![parse_js_function_item(
                JsLikeFunctionKind::Function,
                node,
                node,
                docs,
                flavor,
                None,
            )]
        }
        "method_definition" | "method_signature" | "abstract_method_signature" => {
            vec![parse_js_function_item(
                JsLikeFunctionKind::Method,
                node,
                node,
                docs,
                flavor,
                None,
            )]
        }
        "field_definition" | "public_field_definition" => {
            if let Some(item) = parse_js_callable_field_item(node, docs, flavor) {
                vec![item]
            } else if flavor == JsLikeFlavor::TypeScript {
                vec![parse_ts_field_item(node, docs, "field")]
            } else {
                Vec::new()
            }
        }
        "property_signature" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_ts_field_item(node, docs, "property")]
        }
        "call_signature" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_ts_unnamed_signature_item(
                node,
                docs,
                "call",
                "call signature",
            )]
        }
        "construct_signature" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_ts_unnamed_signature_item(
                node,
                docs,
                "new",
                "construct signature",
            )]
        }
        "index_signature" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_ts_unnamed_signature_item(
                node,
                docs,
                "index",
                "index signature",
            )]
        }
        "lexical_declaration" | "variable_declaration" => {
            parse_js_variable_declaration_items(node, node, docs, flavor)
        }
        "export_statement" => parse_js_export_items(node, docs, flavor),
        "expression_statement" => parse_js_expression_statement_items(node, docs, flavor),
        "interface_declaration" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_interface_item(node, node, docs, flavor)]
        }
        "enum_declaration" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_simple_item(
                AstItemKind::Enum,
                js_enum_keyword(node),
                node,
                node,
                docs,
                None,
            )]
        }
        "type_alias_declaration" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_simple_item(
                AstItemKind::TypeAlias,
                "type",
                node,
                node,
                docs,
                None,
            )]
        }
        "module" | "internal_module" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_module_item(node, node, docs, flavor)]
        }
        "ambient_declaration" if flavor == JsLikeFlavor::TypeScript => {
            parse_ts_ambient_items(node, node, docs, flavor)
        }
        _ => Vec::new(),
    }
}

fn parse_js_declaration_items(
    render_node: Node<'_>,
    declaration: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    match declaration.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            vec![parse_js_class_item(
                render_node,
                declaration,
                docs,
                flavor,
                None,
            )]
        }
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            vec![parse_js_function_item(
                JsLikeFunctionKind::Function,
                render_node,
                declaration,
                docs,
                flavor,
                None,
            )]
        }
        "lexical_declaration" | "variable_declaration" => {
            parse_js_variable_declaration_items(render_node, declaration, docs, flavor)
        }
        "interface_declaration" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_interface_item(
                render_node,
                declaration,
                docs,
                flavor,
            )]
        }
        "enum_declaration" if flavor == JsLikeFlavor::TypeScript => vec![parse_js_simple_item(
            AstItemKind::Enum,
            js_enum_keyword(declaration),
            render_node,
            declaration,
            docs,
            None,
        )],
        "type_alias_declaration" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_simple_item(
                AstItemKind::TypeAlias,
                "type",
                render_node,
                declaration,
                docs,
                None,
            )]
        }
        "module" | "internal_module" if flavor == JsLikeFlavor::TypeScript => {
            vec![parse_js_module_item(render_node, declaration, docs, flavor)]
        }
        "ambient_declaration" if flavor == JsLikeFlavor::TypeScript => {
            parse_ts_ambient_items(render_node, declaration, docs, flavor)
        }
        _ => Vec::new(),
    }
}

fn parse_js_export_items(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let is_typescript = flavor == JsLikeFlavor::TypeScript;
    if is_typescript
        && direct_child_with_kind(node, "as").is_some()
        && direct_child_with_kind(node, "namespace").is_some()
    {
        return vec![parse_ts_export_binding_item(
            node,
            docs,
            "export-as-namespace",
        )];
    }
    if is_typescript && direct_child_with_kind(node, "=").is_some() {
        return vec![parse_ts_export_binding_item(node, docs, "export=")];
    }

    if let Some(declaration) = node.child_by_field_name("declaration") {
        return parse_js_declaration_items(node, declaration, docs, flavor);
    }

    if let Some(value) = node.child_by_field_name("value") {
        let value = unwrap_js_parenthesized_expression(value);
        return match value.kind() {
            "arrow_function" | "function_expression" | "generator_function" => {
                vec![parse_js_function_item(
                    JsLikeFunctionKind::Function,
                    node,
                    value,
                    docs,
                    flavor,
                    Some("default".to_owned()),
                )]
            }
            "class" => vec![parse_js_class_item(
                node,
                value,
                docs,
                flavor,
                Some("default".to_owned()),
            )],
            "object" => vec![parse_js_object_item(
                node,
                value,
                docs,
                flavor,
                "default".to_owned(),
            )],
            _ if is_typescript && direct_child_with_kind(node, "default").is_some() => {
                vec![parse_ts_export_binding_item(node, docs, "default")]
            }
            _ => parse_js_assignment_like_item(node, value, docs, flavor)
                .into_iter()
                .collect(),
        };
    }

    if is_typescript && direct_child_with_kind(node, "default").is_some() {
        return vec![parse_ts_export_binding_item(node, docs, "default")];
    }

    Vec::new()
}

fn parse_ts_export_binding_item(node: Node<'_>, docs: &JsDocContext<'_>, name: &str) -> AstItem {
    let source = docs.source;
    let text = trimmed_node_text(node, source);
    AstItem {
        kind: AstItemKind::Use,
        name: Some(name.to_owned()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: docs.leading_item_docs(node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: text.clone(),
        signature: Some(text),
        body: None,
        children: Vec::new(),
    }
}

fn parse_js_expression_statement_items(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let mut cursor = node.walk();
    let Some(value) = node.named_children(&mut cursor).next() else {
        return Vec::new();
    };
    let value = unwrap_js_parenthesized_expression(value);
    if flavor == JsLikeFlavor::TypeScript && value.kind() == "internal_module" {
        return vec![parse_js_module_item(node, value, docs, flavor)];
    }
    parse_js_assignment_like_item(node, value, docs, flavor)
        .into_iter()
        .collect()
}

fn parse_ts_ambient_items(
    render_node: Node<'_>,
    ambient_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let mut cursor = ambient_node.walk();
    let Some(declaration) = ambient_node.named_children(&mut cursor).next() else {
        return Vec::new();
    };
    if declaration.kind() == "statement_block" {
        return vec![parse_js_module_item_with(
            render_node,
            ambient_node,
            Some(declaration),
            docs,
            flavor,
            Some("global".to_owned()),
            Some("module"),
        )];
    }
    if matches!(
        declaration.kind(),
        "lexical_declaration" | "variable_declaration"
    ) {
        return parse_ts_ambient_variable_items(render_node, declaration, docs);
    }
    parse_js_declaration_items(render_node, declaration, docs, flavor)
}

fn parse_js_assignment_like_item(
    render_node: Node<'_>,
    value: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Option<AstItem> {
    if value.kind() != "assignment_expression" {
        return None;
    }
    let source = docs.source;
    let name = assignment_target_name(value, source)?;
    let definition = unwrap_js_parenthesized_expression(value.child_by_field_name("right")?);
    match definition.kind() {
        "arrow_function" | "function_expression" | "generator_function" => {
            Some(parse_js_function_item(
                JsLikeFunctionKind::Function,
                render_node,
                definition,
                docs,
                flavor,
                Some(name),
            ))
        }
        "class" => Some(parse_js_class_item(
            render_node,
            definition,
            docs,
            flavor,
            Some(name),
        )),
        "object" => Some(parse_js_object_item(
            render_node,
            definition,
            docs,
            flavor,
            name,
        )),
        _ => None,
    }
}

fn parse_js_variable_declaration_items(
    render_node: Node<'_>,
    declaration_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let mut cursor = declaration_node.walk();
    let declarators: Vec<_> = declaration_node
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "variable_declarator")
        .collect();
    let owns_whole_declaration = declarators.len() == 1;
    let mut items = Vec::new();
    for declarator in declarators {
        if let Some(mut item) =
            parse_js_variable_declarator_item(render_node, declarator, docs, flavor)
        {
            if !owns_whole_declaration {
                item.location.is_edit_ready = false;
            }
            if !items.is_empty() {
                item.docs = None;
            }
            items.push(item);
        }
    }
    items
}

fn parse_js_variable_declarator_item(
    render_node: Node<'_>,
    declarator: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Option<AstItem> {
    let source = docs.source;
    let name = match declarator.child_by_field_name("name") {
        Some(name) if name.kind() == "identifier" => trimmed_node_text(name, source),
        _ => return None,
    };
    let Some(value) = declarator.child_by_field_name("value") else {
        return (flavor == JsLikeFlavor::TypeScript).then(|| {
            parse_ts_variable_declarator_without_value(render_node, declarator, docs, name)
        });
    };
    let value = unwrap_js_parenthesized_expression(value);
    let mut item = match value.kind() {
        "arrow_function" | "function_expression" | "generator_function" => {
            Some(parse_js_function_item(
                JsLikeFunctionKind::Function,
                render_node,
                value,
                docs,
                flavor,
                Some(name),
            ))
        }
        "class" => Some(parse_js_class_item(
            render_node,
            value,
            docs,
            flavor,
            Some(name),
        )),
        "object" => Some(parse_js_object_item(render_node, value, docs, flavor, name)),
        _ => None,
    }?;
    item.signature = Some(js_variable_declarator_signature(
        render_node,
        declarator,
        value,
        source,
    ));
    Some(item)
}

fn parse_ts_variable_declarator_without_value(
    render_node: Node<'_>,
    declarator: Node<'_>,
    docs: &JsDocContext<'_>,
    name: String,
) -> AstItem {
    let source = docs.source;
    let declaration = declarator
        .parent()
        .expect("a variable declarator always has a declaration parent");
    let keyword = ["const", "let", "var"]
        .into_iter()
        .find_map(|kind| direct_child_with_kind(declaration, kind))
        .map(|node| trimmed_node_text(node, source))
        .unwrap_or_else(|| "const".to_owned());
    AstItem {
        kind: AstItemKind::Const,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: docs.leading_item_docs(render_node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{keyword} {name}"),
        signature: Some(trimmed_node_text(render_node, source)),
        body: Some(trimmed_node_text(render_node, source)),
        children: Vec::new(),
    }
}

fn js_variable_declarator_signature(
    render_node: Node<'_>,
    declarator: Node<'_>,
    value: Node<'_>,
    source: &str,
) -> String {
    let declaration = declarator
        .parent()
        .expect("a variable declarator always has a declaration parent");
    let keyword = ["const", "let", "var"]
        .into_iter()
        .find_map(|kind| direct_child_with_kind(declaration, kind))
        .map(|node| trimmed_node_text(node, source))
        .unwrap_or_else(|| "const".to_owned());
    let export_prefix = source_fragment(source, render_node.start_byte(), declaration.start_byte());
    let declaration_prefix = if export_prefix.is_empty() {
        keyword
    } else {
        format!("{export_prefix} {keyword}")
    };
    let signature = signature_text_with_body(
        declarator,
        value.child_by_field_name("body").or_else(|| {
            (value.kind() == "object")
                .then(|| direct_child_with_kind(value, "{"))
                .flatten()
        }),
        source,
    );
    format!("{declaration_prefix} {signature}")
}

fn parse_js_callable_field_item(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Option<AstItem> {
    let source = docs.source;
    let name_node = node
        .child_by_field_name("property")
        .or_else(|| node.child_by_field_name("name"))?;
    let name = js_property_name(name_node, source);
    let value = unwrap_js_parenthesized_expression(node.child_by_field_name("value")?);
    match value.kind() {
        "arrow_function" | "function_expression" | "generator_function" => {
            Some(parse_js_function_item(
                JsLikeFunctionKind::Method,
                node,
                value,
                docs,
                flavor,
                Some(name),
            ))
        }
        _ => None,
    }
}

fn parse_ts_field_item(node: Node<'_>, docs: &JsDocContext<'_>, default_kind: &str) -> AstItem {
    let source = docs.source;
    let name_node = node
        .child_by_field_name("property")
        .or_else(|| node.child_by_field_name("name"));
    let name = name_node
        .map(|name| js_property_name(name, source))
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let kind = if direct_child_with_kind(node, "accessor").is_some() {
        "accessor"
    } else {
        default_kind
    };
    let owned_start = js_declaration_start_node(node);
    AstItem {
        kind: AstItemKind::Field,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_js_node(owned_start, node, source),
        docs: docs.leading_item_docs(owned_start),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{kind} {name}"),
        signature: Some(js_owned_node_text(owned_start, node, source)),
        body: None,
        children: Vec::new(),
    }
}

fn parse_ts_constructor_parameter_properties(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
) -> Vec<AstItem> {
    if !matches!(
        node.kind(),
        "method_definition" | "method_signature" | "abstract_method_signature"
    ) || node
        .child_by_field_name("name")
        .is_none_or(|name| trimmed_node_text(name, docs.source) != "constructor")
    {
        return Vec::new();
    }
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .filter(|parameter| {
            direct_child_with_kind(*parameter, "accessibility_modifier").is_some()
                || direct_child_with_kind(*parameter, "override_modifier").is_some()
                || direct_child_with_kind(*parameter, "readonly").is_some()
        })
        .filter_map(|parameter| {
            let name_node = parameter
                .child_by_field_name("name")
                .or_else(|| parameter.child_by_field_name("pattern"))?;
            let name = trimmed_node_text(name_node, docs.source);
            Some(AstItem {
                kind: AstItemKind::Field,
                name: Some(name.clone()),
                associated_type: None,
                location: location_for_node(parameter, docs.source),
                docs: None,
                inner_docs: None,
                attributes: None,
                source_preamble: None,
                summary: format!("property {name}"),
                signature: Some(trimmed_node_text(parameter, docs.source)),
                body: None,
                children: Vec::new(),
            })
        })
        .collect()
}

fn parse_ts_unnamed_signature_item(
    node: Node<'_>,
    docs: &JsDocContext<'_>,
    name: &str,
    summary: &str,
) -> AstItem {
    let source = docs.source;
    let owned_start = js_declaration_start_node(node);
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name.to_owned()),
        associated_type: None,
        location: location_for_js_node(owned_start, node, source),
        docs: docs.leading_item_docs(owned_start),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: summary.to_owned(),
        signature: Some(js_owned_node_text(owned_start, node, source)),
        body: Some(js_owned_node_text(owned_start, node, source)),
        children: Vec::new(),
    }
}

fn parse_ts_ambient_variable_items(
    render_node: Node<'_>,
    declaration_node: Node<'_>,
    docs: &JsDocContext<'_>,
) -> Vec<AstItem> {
    let source = docs.source;
    let keyword = ["const", "let", "var"]
        .into_iter()
        .find_map(|kind| direct_child_with_kind(declaration_node, kind))
        .map(|node| trimmed_node_text(node, source))
        .unwrap_or_else(|| "const".to_owned());
    let mut cursor = declaration_node.walk();
    declaration_node
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "variable_declarator")
        .filter_map(|declarator| {
            let name_node = declarator.child_by_field_name("name")?;
            let name = trimmed_node_text(name_node, source);
            Some(AstItem {
                kind: AstItemKind::Const,
                name: Some(name.clone()),
                associated_type: None,
                location: location_for_node(render_node, source),
                docs: docs.leading_item_docs(render_node),
                inner_docs: None,
                attributes: None,
                source_preamble: None,
                summary: format!("{keyword} {name}"),
                signature: Some(trimmed_node_text(render_node, source)),
                body: Some(trimmed_node_text(render_node, source)),
                children: Vec::new(),
            })
        })
        .enumerate()
        .map(|(index, mut item)| {
            if index > 0 {
                item.docs = None;
            }
            item
        })
        .collect()
}

fn parse_js_object_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
    name: String,
) -> AstItem {
    let source = docs.source;
    let open_brace = direct_child_with_kind(definition_node, "{");
    AstItem {
        kind: AstItemKind::Module,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: docs.leading_item_docs(render_node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("object {name}"),
        signature: Some(signature_text_with_body(render_node, open_brace, source)),
        body: Some(trimmed_node_text(render_node, source)),
        children: collect_js_object_items(definition_node, docs, flavor),
    }
}

fn collect_js_object_items(
    object: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> Vec<AstItem> {
    let source = docs.source;
    let mut cursor = object.walk();
    object
        .named_children(&mut cursor)
        .filter_map(|child| match child.kind() {
            "method_definition" => Some(parse_js_function_item(
                JsLikeFunctionKind::Method,
                child,
                child,
                docs,
                flavor,
                None,
            )),
            "pair" => {
                let key = child.child_by_field_name("key")?;
                let name = js_property_name(key, source);
                let value = unwrap_js_parenthesized_expression(child.child_by_field_name("value")?);
                match value.kind() {
                    "arrow_function" | "function_expression" | "generator_function" => {
                        Some(parse_js_function_item(
                            JsLikeFunctionKind::Function,
                            child,
                            value,
                            docs,
                            flavor,
                            Some(name),
                        ))
                    }
                    "class" => Some(parse_js_class_item(child, value, docs, flavor, Some(name))),
                    "object" => Some(parse_js_object_item(child, value, docs, flavor, name)),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn parse_js_function_item(
    kind: JsLikeFunctionKind,
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
    name_override: Option<String>,
) -> AstItem {
    let source = docs.source;
    let owned_start = js_declaration_start_node(render_node);
    let name = name_override.unwrap_or_else(|| {
        child_text_by_field(definition_node, "name", source)
            .unwrap_or_else(|| "<anonymous>".to_owned())
    });
    let body = definition_node.child_by_field_name("body");
    let signature = js_signature_text_with_body(owned_start, render_node, body, source);
    AstItem {
        kind: AstItemKind::Function,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_js_node(owned_start, render_node, source),
        docs: docs.leading_item_docs(owned_start),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: summarize_js_function(
            kind,
            &name,
            js_function_modifiers(definition_node, render_node),
        ),
        signature: Some(signature),
        body: Some(js_owned_node_text(owned_start, render_node, source)),
        children: body
            .filter(|body| body.kind() == "statement_block")
            .map(|body| collect_js_like_supported_items(body, docs, flavor))
            .unwrap_or_default(),
    }
}

fn parse_js_class_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
    name_override: Option<String>,
) -> AstItem {
    let source = docs.source;
    let owned_start = js_declaration_start_node(render_node);
    let name = name_override.unwrap_or_else(|| {
        child_text_by_field(definition_node, "name", source)
            .unwrap_or_else(|| "<anonymous>".to_owned())
    });
    let body = definition_node.child_by_field_name("body");
    AstItem {
        kind: AstItemKind::Class,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_js_node(owned_start, render_node, source),
        docs: docs.leading_item_docs(owned_start),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: summarize_js_class(&name, definition_node),
        signature: Some(js_signature_text_with_body(
            owned_start,
            render_node,
            body,
            source,
        )),
        body: Some(js_owned_node_text(owned_start, render_node, source)),
        children: body
            .map(|body| collect_js_like_supported_items(body, docs, flavor))
            .unwrap_or_default(),
    }
}

fn parse_js_interface_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> AstItem {
    let source = docs.source;
    let name = child_text_by_field(definition_node, "name", source)
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let body = definition_node.child_by_field_name("body");
    AstItem {
        kind: AstItemKind::Interface,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: docs.leading_item_docs(render_node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("interface {name}"),
        signature: Some(signature_text_with_body(render_node, body, source)),
        body: Some(trimmed_node_text(render_node, source)),
        children: body
            .map(|body| collect_js_like_supported_items(body, docs, flavor))
            .unwrap_or_default(),
    }
}

fn parse_js_module_item(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
) -> AstItem {
    parse_js_module_item_with(
        render_node,
        definition_node,
        definition_node.child_by_field_name("body"),
        docs,
        flavor,
        None,
        None,
    )
}

fn parse_js_module_item_with(
    render_node: Node<'_>,
    definition_node: Node<'_>,
    body: Option<Node<'_>>,
    docs: &JsDocContext<'_>,
    flavor: JsLikeFlavor,
    name_override: Option<String>,
    keyword_override: Option<&str>,
) -> AstItem {
    let source = docs.source;
    let name = name_override.unwrap_or_else(|| {
        definition_node
            .child_by_field_name("name")
            .map(|name| js_module_name(name, source))
            .unwrap_or_else(|| "<anonymous>".to_owned())
    });
    let signature = signature_text_with_body(render_node, body, source);
    let keyword = keyword_override.unwrap_or(match definition_node.kind() {
        "internal_module" => "namespace",
        _ => "module",
    });
    AstItem {
        kind: AstItemKind::Module,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: docs.leading_item_docs(render_node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{keyword} {name}"),
        signature: Some(signature),
        body: Some(trimmed_node_text(render_node, source)),
        children: body
            .map(|body| collect_js_like_supported_items(body, docs, flavor))
            .unwrap_or_default(),
    }
}

fn parse_js_simple_item(
    kind: AstItemKind,
    keyword: &str,
    render_node: Node<'_>,
    definition_node: Node<'_>,
    docs: &JsDocContext<'_>,
    name_override: Option<String>,
) -> AstItem {
    let source = docs.source;
    let name = name_override.unwrap_or_else(|| {
        child_text_by_field(definition_node, "name", source)
            .unwrap_or_else(|| "<anonymous>".to_owned())
    });
    let body = definition_node.child_by_field_name("body");
    AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(render_node, source),
        docs: docs.leading_item_docs(render_node),
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{keyword} {name}"),
        signature: Some(signature_text_with_body(render_node, body, source)),
        body: Some(trimmed_node_text(render_node, source)),
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
    let preamble = docs.item_preamble(node);
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("{keyword} {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children: Vec::new(),
    }
}

fn parse_trait_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
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
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("trait {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children,
    }
}

fn parse_impl_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let target =
        child_text_by_field(node, "type", source).unwrap_or_else(|| "<unknown>".to_owned());
    let associated_type = node
        .child_by_field_name("type")
        .and_then(|target| extract_nominal_type(target, source));
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
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary,
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children,
    }
}

fn parse_mod_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let children = node
        .child_by_field_name("body")
        .map(|body| collect_supported_items(body, docs))
        .unwrap_or_default();
    let inner_docs = node
        .child_by_field_name("body")
        .and_then(|body| docs.inner_docs(body));
    AstItem {
        kind: AstItemKind::Module,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("mod {name}"),
        signature: Some(signature_text(node, source)),
        body: Some(trimmed_node_text(node, source)),
        children,
    }
}

fn parse_use_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let text = trimmed_node_text(node, source);
    AstItem {
        kind: AstItemKind::Use,
        name: child_text_by_field(node, "argument", source),
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: text.clone(),
        signature: Some(text),
        body: None,
        children: Vec::new(),
    }
}

fn parse_macro_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    AstItem {
        kind: AstItemKind::Macro,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("macro_rules! {name}"),
        signature: Some(format!("macro_rules! {name}")),
        body: Some(trimmed_node_text(node, source)),
        children: Vec::new(),
    }
}

fn parse_macro_invocation(
    render_node: Node<'_>,
    invocation_node: Node<'_>,
    docs: &RustDocContext<'_>,
) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(render_node);
    let name = child_text_by_field(invocation_node, "macro", source)
        .unwrap_or_else(|| "<anonymous>".to_owned());
    let text = trimmed_node_text(render_node, source);
    AstItem {
        kind: AstItemKind::MacroInvocation,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_rust_node(render_node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: format!("macro invocation {name}!"),
        signature: Some(text),
        body: None,
        children: Vec::new(),
    }
}

fn parse_foreign_mod_item(node: Node<'_>, docs: &RustDocContext<'_>) -> AstItem {
    let source = docs.source;
    let preamble = docs.item_preamble(node);
    let body = node.child_by_field_name("body");
    let signature = signature_text_with_body(node, body, source);
    AstItem {
        kind: AstItemKind::ForeignBlock,
        name: None,
        associated_type: None,
        location: location_for_rust_node(node, &preamble, source),
        docs: preamble.docs.clone(),
        inner_docs: None,
        attributes: preamble.attributes.clone(),
        source_preamble: preamble.source_text.clone(),
        summary: signature.clone(),
        signature: Some(signature),
        body: Some(trimmed_node_text(node, source)),
        children: body
            .map(|body| collect_supported_items(body, docs))
            .unwrap_or_default(),
    }
}

#[derive(Debug, Default)]
struct RustItemPreamble {
    start: Option<Point>,
    start_byte: Option<usize>,
    docs: Option<String>,
    attributes: Option<String>,
    source_text: Option<String>,
}

struct RustDocContext<'a> {
    source: &'a str,
}

impl<'a> RustDocContext<'a> {
    fn new(source: &'a str) -> Self {
        Self { source }
    }

    fn inner_docs(&self, container: Node<'_>) -> Option<String> {
        let mut cursor = container.walk();
        let mut fragments = Vec::new();
        for child in container.named_children(&mut cursor) {
            let text = trimmed_node_text(child, self.source);
            if child.kind() == "inner_attribute_item" && is_rust_doc_attribute(&text, true) {
                fragments.push(text);
                continue;
            }
            if matches!(child.kind(), "inner_attribute_item" | "shebang") {
                continue;
            }
            if is_rust_inner_doc_comment(&text) {
                fragments.push(text);
                continue;
            }
            if is_rust_comment_node(child) && !is_rust_outer_doc_comment(&text) {
                continue;
            }
            break;
        }
        (!fragments.is_empty()).then(|| fragments.join("\n"))
    }

    fn item_preamble(&self, node: Node<'_>) -> RustItemPreamble {
        let mut current = node.prev_named_sibling();
        let mut anchor = node.start_byte();
        let mut crossed_blank_line = false;
        let mut start = None;
        let mut start_byte = None;
        let mut docs = Vec::new();
        let mut attributes = Vec::new();
        let mut source_fragments = Vec::new();

        while let Some(sibling) = current {
            let text = trimmed_node_text(sibling, self.source);
            crossed_blank_line |= rust_gap_has_blank_line(&self.source[sibling.end_byte()..anchor]);
            if sibling.kind() == "attribute_item" && is_rust_doc_attribute(&text, false) {
                docs.push(text.clone());
            } else if sibling.kind() == "attribute_item" {
                attributes.push(text.clone());
            } else if is_rust_outer_doc_comment(&text) {
                docs.push(text.clone());
            } else if is_rust_inner_doc_comment(&text) {
                break;
            } else if is_rust_comment_node(sibling) {
                if crossed_blank_line || rust_comment_trails_prior_code(sibling) {
                    break;
                }
                attributes.push(text.clone());
            } else {
                break;
            }
            source_fragments.push(text);
            start = Some(sibling.start_position());
            start_byte = Some(sibling.start_byte());
            anchor = sibling.start_byte();
            current = sibling.prev_named_sibling();
        }

        docs.reverse();
        attributes.reverse();
        source_fragments.reverse();
        RustItemPreamble {
            start,
            start_byte,
            docs: (!docs.is_empty()).then(|| docs.join("\n")),
            attributes: (!attributes.is_empty()).then(|| attributes.join("\n")),
            source_text: (!source_fragments.is_empty()).then(|| source_fragments.join("\n")),
        }
    }
}

struct PythonDocContext<'a> {
    source: &'a str,
}

impl<'a> PythonDocContext<'a> {
    fn root_module_docs(&self, root: Node<'_>) -> Option<String> {
        self.docstring_for_body(root)
    }

    fn docstring_for_body(&self, body: Node<'_>) -> Option<String> {
        let mut cursor = body.walk();
        let first_statement = body
            .named_children(&mut cursor)
            .find(|child| child.kind() != "comment")?;
        extract_python_docstring(first_statement, self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JsCommentRange {
    start: usize,
    end: usize,
}

struct JsDocContext<'a> {
    source: &'a str,
    comments: Vec<JsCommentRange>,
    root_comments: Vec<JsCommentRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoItemPreamble {
    start: usize,
    text: String,
    directives: Option<String>,
}

struct GoDocContext<'a> {
    source: &'a str,
    comments: Vec<JsCommentRange>,
    root_comments: Vec<JsCommentRange>,
}

impl<'a> GoDocContext<'a> {
    fn new(source: &'a str, root: Node<'_>) -> Self {
        let mut comments = Vec::new();
        collect_js_comment_ranges(root, &mut comments);
        let package_start = {
            let mut cursor = root.walk();
            root.named_children(&mut cursor)
                .find(|child| child.kind() == "package_clause")
                .map(|package| package.start_byte())
        };
        let root_comments = package_start.map_or_else(Vec::new, |package_start| {
            comments
                .iter()
                .copied()
                .filter(|comment| comment.end <= package_start)
                .collect()
        });
        Self {
            source,
            comments,
            root_comments,
        }
    }

    fn root_module_docs(&self) -> Option<String> {
        let first = self.root_comments.first()?;
        let last = self.root_comments.last()?;
        Some(trimmed_text(self.source, first.start, last.end))
    }

    fn leading_item_preamble(&self, node: Node<'_>) -> Option<GoItemPreamble> {
        let owned = leading_comment_ranges(self.source, &self.comments, node);
        let first = owned.first()?;
        let last = owned.last()?;
        let text = trimmed_text(self.source, first.start, last.end);
        let directives = text
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("//go:") || line.starts_with("//line ")
            })
            .collect::<Vec<_>>();
        let directives = (!directives.is_empty()).then(|| directives.join("\n"));
        Some(GoItemPreamble {
            start: first.start,
            text,
            directives,
        })
    }
}

fn merge_go_preambles(
    context: Option<&GoItemPreamble>,
    owned: Option<&GoItemPreamble>,
) -> Option<GoItemPreamble> {
    match (context, owned) {
        (Some(context), Some(owned)) => {
            let directives = [context.directives.as_deref(), owned.directives.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
            Some(GoItemPreamble {
                start: owned.start,
                text: format!("{}\n{}", context.text, owned.text),
                directives: (!directives.is_empty()).then_some(directives),
            })
        }
        (Some(context), None) => Some(GoItemPreamble {
            start: context.start,
            text: context.text.clone(),
            directives: context.directives.clone(),
        }),
        (None, Some(owned)) => Some(owned.clone()),
        (None, None) => None,
    }
}

impl<'a> JsDocContext<'a> {
    fn new(source: &'a str, root: Node<'_>) -> Self {
        let mut comments = Vec::new();
        collect_js_comment_ranges(root, &mut comments);
        let root_comments = js_root_comment_ranges(root, source);
        Self {
            source,
            comments,
            root_comments,
        }
    }

    fn root_module_docs(&self) -> Option<String> {
        let first = self.root_comments.first()?;
        let last = self.root_comments.last()?;
        Some(trimmed_text(self.source, first.start, last.end))
    }

    fn leading_item_docs(&self, node: Node<'_>) -> Option<String> {
        let owned = leading_comment_ranges(self.source, &self.comments, node);
        let first = owned.first()?;
        let last = owned.last()?;
        Some(trimmed_text(self.source, first.start, last.end))
    }
}

fn leading_comment_ranges(
    source: &str,
    comments: &[JsCommentRange],
    node: Node<'_>,
) -> Vec<JsCommentRange> {
    let mut anchor = node.start_byte();
    let mut owned = Vec::new();

    while let Some(comment) = comments
        .iter()
        .rev()
        .find(|comment| comment.end <= anchor)
        .copied()
    {
        let gap = &source[comment.end..anchor];
        if !gap.trim().is_empty() || js_gap_has_blank_line(gap) {
            break;
        }
        let line_start = source[..comment.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        if !source[line_start..comment.start].trim().is_empty() {
            break;
        }
        owned.push(comment);
        anchor = comment.start;
    }

    owned.reverse();
    owned
}

fn collect_js_comment_ranges(node: Node<'_>, comments: &mut Vec<JsCommentRange>) {
    if node.kind() == "comment" {
        comments.push(JsCommentRange {
            start: node.start_byte(),
            end: node.end_byte(),
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_comment_ranges(child, comments);
    }
}

fn js_root_comment_ranges(root: Node<'_>, source: &str) -> Vec<JsCommentRange> {
    let mut cursor = root.walk();
    let leading: Vec<_> = root
        .named_children(&mut cursor)
        .skip_while(|child| child.kind() == "hash_bang_line")
        .take_while(|child| child.kind() == "comment")
        .map(|comment| JsCommentRange {
            start: comment.start_byte(),
            end: comment.end_byte(),
        })
        .collect();
    if leading.is_empty() {
        return leading;
    }

    let mut cursor = root.walk();
    let next_syntax = root
        .named_children(&mut cursor)
        .find(|child| !matches!(child.kind(), "hash_bang_line" | "comment"));
    let Some(next_syntax) = next_syntax else {
        return leading;
    };
    let last = leading.last().expect("leading comments are non-empty");
    if js_gap_has_blank_line(&source[last.end..next_syntax.start_byte()]) {
        return leading;
    }

    let mut owned_group_start = leading.len() - 1;
    while owned_group_start > 0 {
        let previous = leading[owned_group_start - 1];
        let current = leading[owned_group_start];
        if js_gap_has_blank_line(&source[previous.end..current.start]) {
            break;
        }
        owned_group_start -= 1;
    }
    leading[..owned_group_start].to_vec()
}

fn js_gap_has_blank_line(gap: &str) -> bool {
    let normalized = gap.replace("\r\n", "\n").replace('\r', "\n");
    normalized.bytes().filter(|byte| *byte == b'\n').count() >= 2
}

fn is_rust_outer_doc_comment(text: &str) -> bool {
    text.starts_with("///") && !text.starts_with("////")
        || text.starts_with("/**") && !text.starts_with("/***")
}

fn is_rust_inner_doc_comment(text: &str) -> bool {
    text.starts_with("//!") || text.starts_with("/*!")
}

fn is_rust_doc_attribute(text: &str, inner: bool) -> bool {
    let prefix = if inner { "#![" } else { "#[" };
    let Some(contents) = text.strip_prefix(prefix) else {
        return false;
    };
    let Some(remainder) = contents.trim_start().strip_prefix("doc") else {
        return false;
    };
    remainder.trim_start().starts_with('=')
}

fn is_rust_comment_node(node: Node<'_>) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

fn rust_comment_trails_prior_code(comment: Node<'_>) -> bool {
    let row = comment.start_position().row;
    let mut previous = comment.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.end_position().row != row {
            return false;
        }
        if is_rust_comment_node(sibling) {
            previous = sibling.prev_named_sibling();
            continue;
        }
        let is_owned_preamble = sibling.kind() == "attribute_item";
        return !is_owned_preamble;
    }
    false
}

fn rust_gap_has_blank_line(gap: &str) -> bool {
    let normalized = gap.replace("\r\n", "\n").replace('\r', "\n");
    normalized.bytes().filter(|byte| *byte == b'\n').count() >= 2
}

fn go_declaration_is_grouped(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| matches!(child.kind(), "(" | "var_spec_list"))
}

fn go_declaration_specs(node: Node<'_>, is_const: bool) -> Vec<Node<'_>> {
    let expected = if is_const { "const_spec" } else { "var_spec" };
    let mut specs = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == expected {
            specs.push(child);
        } else if child.kind() == "var_spec_list" {
            let mut list_cursor = child.walk();
            specs.extend(
                child
                    .named_children(&mut list_cursor)
                    .filter(|spec| spec.kind() == expected),
            );
        }
    }
    specs
}

fn go_spec_names(node: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = node.walk();
    node.children_by_field_name("name", &mut cursor)
        .filter(|name| matches!(name.kind(), "identifier" | "type_identifier"))
        .map(|name| trimmed_node_text(name, source))
        .collect()
}

fn go_prefixed_spec(keyword: &str, owned_node: Node<'_>, spec: Node<'_>, source: &str) -> String {
    if owned_node.id() == spec.id() {
        format!("{keyword} {}", trimmed_node_text(spec, source))
    } else {
        trimmed_node_text(owned_node, source)
    }
}

fn extract_go_receiver_type(receiver: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = receiver.walk();
    receiver.named_children(&mut cursor).find_map(|parameter| {
        parameter
            .child_by_field_name("type")
            .and_then(|node| extract_go_nominal_type(node, source))
            .or_else(|| extract_go_nominal_type(parameter, source))
    })
}

fn extract_go_nominal_type(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(trimmed_node_text(node, source)),
        "qualified_type" => node
            .child_by_field_name("name")
            .map(|name| trimmed_node_text(name, source)),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| extract_go_nominal_type(inner, source)),
        "pointer_type" | "parameter_declaration" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                return extract_go_nominal_type(type_node, source);
            }
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find_map(|child| extract_go_nominal_type(child, source))
        }
        _ => None,
    }
}

fn parse_go_type_members(node: Node<'_>, docs: &GoDocContext<'_>) -> Vec<AstItem> {
    match node.kind() {
        "struct_type" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| child.kind() == "field_declaration_list")
                .map(|fields| {
                    let mut fields_cursor = fields.walk();
                    fields
                        .named_children(&mut fields_cursor)
                        .filter(|field| field.kind() == "field_declaration")
                        .flat_map(|field| parse_go_struct_field_items(field, docs))
                        .collect()
                })
                .unwrap_or_default()
        }
        "interface_type" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .filter_map(|member| parse_go_interface_member(member, docs))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn parse_go_struct_field_items(node: Node<'_>, docs: &GoDocContext<'_>) -> Vec<AstItem> {
    let source = docs.source;
    let preamble = docs.leading_item_preamble(node);
    let signature = trimmed_node_text(node, source);
    let mut names = {
        let mut cursor = node.walk();
        node.children_by_field_name("name", &mut cursor)
            .map(|name| trimmed_node_text(name, source))
            .collect::<Vec<_>>()
    };
    if names.is_empty()
        && let Some(type_node) = node.child_by_field_name("type")
        && let Some(name) = extract_go_embedded_name(type_node, source)
    {
        names.push(name);
    }
    names
        .into_iter()
        .map(|name| AstItem {
            kind: AstItemKind::Field,
            name: Some(name.clone()),
            associated_type: None,
            location: location_for_go_node(node, preamble.as_ref(), source),
            docs: preamble.as_ref().map(|preamble| preamble.text.clone()),
            inner_docs: None,
            attributes: preamble
                .as_ref()
                .and_then(|preamble| preamble.directives.clone()),
            source_preamble: preamble.as_ref().map(|preamble| preamble.text.clone()),
            summary: format!("field {name}"),
            signature: Some(signature.clone()),
            body: Some(signature.clone()),
            children: Vec::new(),
        })
        .collect()
}

fn parse_go_interface_member(node: Node<'_>, docs: &GoDocContext<'_>) -> Option<AstItem> {
    let source = docs.source;
    let preamble = docs.leading_item_preamble(node);
    let signature = trimmed_node_text(node, source);
    let (kind, name, summary) = match node.kind() {
        "method_elem" => {
            let name = child_text_by_field(node, "name", source)?;
            (AstItemKind::Function, name.clone(), format!("fn {name}"))
        }
        "type_elem" => {
            let name = extract_go_embedded_name(node, source).unwrap_or_else(|| signature.clone());
            (AstItemKind::Field, name.clone(), format!("type {name}"))
        }
        _ => return None,
    };
    Some(AstItem {
        kind,
        name: Some(name),
        associated_type: None,
        location: location_for_go_node(node, preamble.as_ref(), source),
        docs: preamble.as_ref().map(|preamble| preamble.text.clone()),
        inner_docs: None,
        attributes: preamble
            .as_ref()
            .and_then(|preamble| preamble.directives.clone()),
        source_preamble: preamble.as_ref().map(|preamble| preamble.text.clone()),
        summary,
        signature: Some(signature.clone()),
        body: Some(signature),
        children: Vec::new(),
    })
}

fn extract_go_embedded_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(trimmed_node_text(node, source)),
        "qualified_type" => Some(trimmed_node_text(node, source)),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| extract_go_embedded_name(inner, source)),
        "pointer_type" | "type_elem" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find_map(|child| extract_go_embedded_name(child, source))
        }
        _ => None,
    }
}

fn assignment_target_name(node: Node<'_>, source: &str) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    js_member_path(left, source)
}

fn js_member_path(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "property_identifier" | "private_property_identifier" | "this" | "super" => {
            Some(trimmed_node_text(node, source))
        }
        "member_expression" => {
            let object = js_member_path(node.child_by_field_name("object")?, source)?;
            let property = node.child_by_field_name("property")?;
            Some(format!("{object}.{}", trimmed_node_text(property, source)))
        }
        "subscript_expression" => {
            let object = js_member_path(node.child_by_field_name("object")?, source)?;
            let index = node.child_by_field_name("index")?;
            let index_text = trimmed_node_text(index, source);
            if index.kind() == "string"
                && let Some(property) = js_string_property_name(&index_text)
            {
                Some(format!("{object}.{property}"))
            } else {
                Some(format!("{object}[{index_text}]"))
            }
        }
        _ => None,
    }
}

fn js_property_name(node: Node<'_>, source: &str) -> String {
    let text = trimmed_node_text(node, source);
    if node.kind() == "string" {
        js_string_property_name(&text).unwrap_or(text)
    } else {
        text
    }
}

fn js_module_name(node: Node<'_>, source: &str) -> String {
    let text = trimmed_node_text(node, source);
    if node.kind() == "string" && text.len() >= 2 {
        let quote = text.as_bytes()[0];
        if matches!(quote, b'\'' | b'"') && text.as_bytes()[text.len() - 1] == quote {
            return text[1..text.len() - 1].to_owned();
        }
    }
    text
}

fn js_string_property_name(text: &str) -> Option<String> {
    let quote = text.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') || text.as_bytes().last().copied()? != quote {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    if inner
        .bytes()
        .all(|byte| byte == b'_' || byte == b'$' || byte.is_ascii_alphanumeric())
        && !inner.is_empty()
        && !inner.as_bytes()[0].is_ascii_digit()
    {
        Some(inner.to_owned())
    } else {
        None
    }
}

fn summarize_js_class(name: &str, definition_node: Node<'_>) -> String {
    let summary = if definition_node.kind() == "abstract_class_declaration" {
        "abstract class"
    } else {
        "class"
    };
    format!("{summary} {name}")
}

fn summarize_js_function(
    kind: JsLikeFunctionKind,
    name: &str,
    modifiers: JsLikeFunctionModifiers,
) -> String {
    let mut parts = Vec::new();
    if modifiers.is_static {
        parts.push("static");
    }
    if modifiers.is_abstract {
        parts.push("abstract");
    }
    if modifiers.is_async {
        parts.push("async");
    }
    parts.push(match (kind, modifiers.is_generator) {
        (JsLikeFunctionKind::Function, false) => "function",
        (JsLikeFunctionKind::Function, true) => "function*",
        (JsLikeFunctionKind::Method, false) => "method",
        (JsLikeFunctionKind::Method, true) => "method*",
    });
    format!("{} {name}", parts.join(" "))
}

fn js_function_modifiers(
    definition_node: Node<'_>,
    render_node: Node<'_>,
) -> JsLikeFunctionModifiers {
    JsLikeFunctionModifiers {
        is_async: direct_child_with_kind(definition_node, "async").is_some(),
        is_abstract: definition_node.kind() == "abstract_method_signature",
        is_generator: matches!(
            definition_node.kind(),
            "generator_function" | "generator_function_declaration"
        ) || direct_child_with_kind(definition_node, "*").is_some(),
        is_static: direct_child_with_kind(render_node, "static").is_some(),
    }
}

fn js_enum_keyword(definition_node: Node<'_>) -> &'static str {
    if direct_child_with_kind(definition_node, "const").is_some() {
        "const enum"
    } else {
        "enum"
    }
}

fn unwrap_js_parenthesized_expression(mut node: Node<'_>) -> Node<'_> {
    while node.kind() == "parenthesized_expression" {
        let mut cursor = node.walk();
        let Some(inner) = node.named_children(&mut cursor).next() else {
            break;
        };
        node = inner;
    }
    node
}

fn js_declaration_start_node(node: Node<'_>) -> Node<'_> {
    let mut owned_start = node;
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        match sibling.kind() {
            "decorator" => owned_start = sibling,
            "comment" => {}
            _ => break,
        }
        previous = sibling.prev_named_sibling();
    }
    owned_start
}

fn js_owned_node_text(start_node: Node<'_>, end_node: Node<'_>, source: &str) -> String {
    let end_byte =
        js_declaration_end_node(end_node, source).map_or(end_node.end_byte(), |end| end.end_byte());
    let text = trimmed_text(source, start_node.start_byte(), end_byte);
    dedent_source_block(&text, &source_indent_for_node(start_node, source))
}

fn js_signature_text_with_body(
    start_node: Node<'_>,
    end_node: Node<'_>,
    body: Option<Node<'_>>,
    source: &str,
) -> String {
    if let Some(body) = body {
        let text = source_fragment(source, start_node.start_byte(), body.start_byte());
        dedent_source_block(&text, &source_indent_for_node(start_node, source))
    } else {
        js_owned_node_text(start_node, end_node, source)
    }
}

fn location_for_js_node(
    start_node: Node<'_>,
    end_node: Node<'_>,
    source: &str,
) -> AstLocationRange {
    let owned_end = js_declaration_end_node(end_node, source).unwrap_or(end_node);
    AstLocationRange::from_source_span(
        source,
        start_node.start_position(),
        owned_end.end_position(),
        start_node.start_byte(),
        owned_end.end_byte(),
    )
}

fn js_declaration_end_node<'tree>(node: Node<'tree>, source: &str) -> Option<Node<'tree>> {
    let next = node.next_sibling()?;
    (next.kind() == ";" && source[node.end_byte()..next.start_byte()].trim().is_empty())
        .then_some(next)
}

fn direct_child_with_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn signature_text(node: Node<'_>, source: &str) -> String {
    signature_text_with_body(node, node.child_by_field_name("body"), source)
}

fn signature_text_with_body(node: Node<'_>, body: Option<Node<'_>>, source: &str) -> String {
    if let Some(body) = body {
        return source_fragment(source, node.start_byte(), body.start_byte());
    }
    trimmed_node_text(node, source)
}

fn python_signature_text(render_node: Node<'_>, definition_node: Node<'_>, source: &str) -> String {
    let mut cursor = definition_node.walk();
    let colon = definition_node
        .children(&mut cursor)
        .filter(|child| child.kind() == ":")
        .last();
    let signature = colon
        .map(|colon| source_fragment(source, render_node.start_byte(), colon.end_byte()))
        .unwrap_or_else(|| signature_text(definition_node, source));
    dedent_source_block(&signature, &source_indent_for_node(render_node, source))
}

fn python_source_text(node: Node<'_>, source: &str) -> String {
    dedent_source_block(
        &trimmed_node_text(node, source),
        &source_indent_for_node(node, source),
    )
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

fn location_for_node(node: Node<'_>, source: &str) -> AstLocationRange {
    AstLocationRange::from_source_span(
        source,
        node.start_position(),
        node.end_position(),
        node.start_byte(),
        node.end_byte(),
    )
}

fn location_for_go_node(
    node: Node<'_>,
    preamble: Option<&GoItemPreamble>,
    source: &str,
) -> AstLocationRange {
    let start_byte = preamble.map_or_else(|| node.start_byte(), |preamble| preamble.start);
    AstLocationRange::from_source_span(
        source,
        point_for_byte(source, start_byte),
        node.end_position(),
        start_byte,
        node.end_byte(),
    )
}

fn point_for_byte(source: &str, byte: usize) -> Point {
    let prefix = &source[..byte];
    let row = prefix
        .bytes()
        .filter(|character| *character == b'\n')
        .count();
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, line)| line.len());
    Point::new(row, column)
}

fn location_for_rust_node(
    node: Node<'_>,
    preamble: &RustItemPreamble,
    source: &str,
) -> AstLocationRange {
    AstLocationRange::from_source_span(
        source,
        preamble.start.unwrap_or_else(|| node.start_position()),
        node.end_position(),
        preamble.start_byte.unwrap_or_else(|| node.start_byte()),
        node.end_byte(),
    )
}

fn extract_python_docstring(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "expression_statement" {
        return None;
    }

    let mut cursor = node.walk();
    let value = node.named_children(&mut cursor).next()?;
    let value = unwrap_python_parenthesized_expression(value)?;
    if !is_python_string_literal(value, source) {
        return None;
    }

    Some(trimmed_node_text(value, source))
}

fn unwrap_python_parenthesized_expression(mut node: Node<'_>) -> Option<Node<'_>> {
    while node.kind() == "parenthesized_expression" {
        let mut cursor = node.walk();
        node = node
            .named_children(&mut cursor)
            .find(|child| child.kind() != "comment")?;
    }
    Some(node)
}

fn is_python_string_literal(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "concatenated_string" => {
            let mut cursor = node.walk();
            let mut found_string = false;
            for child in node.named_children(&mut cursor) {
                if child.kind() == "comment" {
                    continue;
                }
                if !is_python_string_literal(child, source) {
                    return false;
                }
                found_string = true;
            }
            found_string
        }
        "string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() == "interpolation")
            {
                return false;
            }
            let text = trimmed_node_text(node, source);
            let prefix = text
                .chars()
                .take_while(|character| !matches!(character, '\'' | '"'))
                .collect::<String>()
                .to_ascii_lowercase();
            !prefix.contains('b') && !prefix.contains('f') && !prefix.contains('t')
        }
        _ => false,
    }
}

fn source_indent_for_node(node: Node<'_>, source: &str) -> String {
    let before = &source[..node.start_byte()];
    let line = before.rsplit_once('\n').map_or(before, |(_, line)| line);
    line.chars()
        .rev()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn extract_nominal_type(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "primitive_type" => Some(trimmed_node_text(node, source)),
        "scoped_type_identifier" => {
            let source_path = trimmed_node_text(node, source);
            Some(canonical_rust_type_path(&source_path))
        }
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| extract_nominal_type(inner, source))
            .or_else(|| {
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find_map(|child| extract_nominal_type(child, source))
            }),
        _ => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .filter(|child| child.kind() != "lifetime")
                .find_map(|child| extract_nominal_type(child, source))
        }
    }
}

fn canonical_rust_type_path(path: &str) -> String {
    path.replace("::", ".")
}

#[derive(Debug)]
struct CompiledAstSelector {
    item_patterns: Vec<CompiledItemSelectorPattern>,
    type_patterns: Vec<CompiledTypeSelectorPattern>,
}

#[derive(Debug)]
struct CompiledItemSelectorPattern {
    matcher: globset::GlobMatcher,
    is_qualified: bool,
}

#[derive(Debug)]
struct CompiledTypeSelectorPattern {
    matcher: globset::GlobMatcher,
    is_qualified: bool,
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
            selector.item_patterns.iter().any(|pattern| {
                pattern.matcher.is_match(path)
                    || !pattern.is_qualified
                        && item.associated_type.is_some()
                        && item
                            .name
                            .as_deref()
                            .is_some_and(|name| pattern.matcher.is_match(name))
            })
        })
        .unwrap_or(false);
    let type_match = (item_supports_type_selection(item) || item.associated_type.is_some())
        && selector.type_patterns.iter().any(|pattern| {
            item_path.is_some_and(|path| pattern.matcher.is_match(path))
                || !pattern.is_qualified
                    && item
                        .associated_type
                        .as_deref()
                        .or(item.name.as_deref())
                        .is_some_and(|name| {
                            pattern.matcher.is_match(name)
                                || name
                                    .rsplit('.')
                                    .next()
                                    .is_some_and(|basename| pattern.matcher.is_match(basename))
                        })
        });
    item_match || type_match
}

fn item_supports_type_selection(item: &AstItem) -> bool {
    matches!(
        item.kind,
        AstItemKind::Class
            | AstItemKind::Interface
            | AstItemKind::Struct
            | AstItemKind::Enum
            | AstItemKind::Union
            | AstItemKind::TypeAlias
            | AstItemKind::Trait
            | AstItemKind::Impl
    )
}

fn selector_path_for_item(item: &AstItem, parent_context: Option<&str>) -> Option<String> {
    match item.kind {
        AstItemKind::Impl => impl_selector_path(item, parent_context),
        AstItemKind::Function if item.associated_type.is_some() => {
            let receiver = item.associated_type.as_deref()?;
            let name = item.name.as_deref()?;
            Some(join_selector_path(
                parent_context,
                &format!("{receiver}.{name}"),
            ))
        }
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
        AstItemKind::Impl => impl_selector_path(item, parent_context),
        _ => selector_path_for_item(item, parent_context),
    }
}

fn impl_selector_path(item: &AstItem, parent_context: Option<&str>) -> Option<String> {
    item.associated_type
        .as_deref()
        .map(|target| resolve_rust_impl_target(target, parent_context))
}

fn resolve_rust_impl_target(target: &str, parent_context: Option<&str>) -> String {
    let mut target_segments = target.split('.').filter(|segment| !segment.is_empty());
    let first = target_segments.next();
    let mut resolved: Vec<&str> = parent_context
        .into_iter()
        .flat_map(|parent| parent.split('.'))
        .filter(|segment| !segment.is_empty())
        .collect();

    match first {
        Some("crate") => resolved.clear(),
        Some("self") => {}
        Some("super") => {
            resolved.pop();
        }
        Some(segment) => resolved.push(segment),
        None => return String::new(),
    }
    for segment in target_segments {
        if segment == "super" {
            resolved.pop();
        } else if segment != "self" {
            resolved.push(segment);
        }
    }
    resolved.join(".")
}

fn join_selector_path(parent: Option<&str>, segment: &str) -> String {
    match parent {
        Some(parent) if !parent.is_empty() => format!("{parent}.{segment}"),
        _ => segment.to_owned(),
    }
}

fn render_item(
    item: &AstItem,
    language: AstLanguage,
    options: AstRenderOptions,
    indent: usize,
    output: &mut String,
) {
    let render_full_body = item.kind.supports_function_bodies() && options.include_function_bodies
        || item.kind.supports_type_bodies() && options.include_type_bodies;
    let render_rust_preamble =
        language == AstLanguage::Rust && (render_full_body || options.include_signatures);
    let render_go_preamble = language == AstLanguage::Go && render_full_body;
    let text = if render_full_body {
        item.body.as_deref().unwrap_or(&item.summary)
    } else if options.include_signatures {
        item.signature.as_deref().unwrap_or(&item.summary)
    } else {
        &item.summary
    };

    let include_separate_docs =
        options.include_docs && !(render_full_body && language == AstLanguage::Python);
    let preamble = if render_rust_preamble || render_go_preamble {
        if include_separate_docs {
            item.source_preamble.as_deref()
        } else {
            item.attributes.as_deref()
        }
    } else if include_separate_docs {
        item.docs.as_deref()
    } else {
        None
    };
    if let Some(preamble) = preamble {
        push_indented_block(output, indent, preamble, None, false);
        output.push('\n');
    }

    push_indented_block(
        output,
        indent,
        text,
        options.include_locations.then(|| item.location.display()),
        language == AstLanguage::Python,
    );

    if !render_full_body {
        if language == AstLanguage::Rust
            && options.include_docs
            && let Some(inner_docs) = item.inner_docs.as_deref()
        {
            output.push('\n');
            push_indented_block(output, indent + 1, inner_docs, None, false);
        }
        for child in &item.children {
            output.push('\n');
            render_item(child, language, options, indent + 1, output);
        }
    }
}

fn push_indented_block(
    output: &mut String,
    indent: usize,
    text: &str,
    location: Option<String>,
    preserve_indentation: bool,
) {
    let prefix = indent_prefix(indent);
    let normalized = if preserve_indentation {
        text.to_owned()
    } else {
        normalize_indentation(text)
    };
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

fn dedent_source_block(text: &str, source_indent: &str) -> String {
    let mut normalized = String::new();
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            normalized.push('\n');
        }
        if index == 0 {
            normalized.push_str(line);
        } else {
            normalized.push_str(line.strip_prefix(source_indent).unwrap_or(line));
        }
    }
    normalized
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
    if lines
        .first()
        .map(|line| line.trim_end().ends_with(':'))
        .unwrap_or(false)
    {
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
    use std::fs;
    use std::path::Path;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        EditProgram, Executor, FileRangeSelection, GenericModification, RangeSet, TextRange,
    };

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

    const PYTHON_SAMPLE: &str = r#"
"""module docs"""

class Greeter:
    """class docs"""

    def greet(self, name: str) -> str:
        """method docs"""

        def normalize(value):
            return value.strip()

        return normalize(name)

@cached
async def run(task):
    return task()
"#;

    const JAVASCRIPT_SAMPLE: &str = r#"
/** module docs */

/** class docs */
class Greeter {
    /** method docs */
    greet(name) {
        function normalize(value) {
            return value.trim();
        }

        return normalize(name);
    }
}

export const run = async (task) => {
    return task();
};
"#;

    const TYPESCRIPT_SAMPLE: &str = r#"
/** module docs */

/** interface docs */
export interface Greeter {
    /** method docs */
    greet(name: string): string;
}

export class Service {
    run(task: string): string {
        const normalize = (value: string) => value.trim();
        return normalize(task);
    }
}

export type Task = { id: string };
export enum Mode {
    Dev,
    Prod,
}
"#;

    const GO_SAMPLE: &str = r#"
// package docs
package sample

// Greeter docs
type Greeter struct {
    Name string
}

type Runner interface {
    // Run docs
    Run(task string) string
}

const DefaultName = "world"
var Count int

func NewGreeter(name string) Greeter {
    return Greeter{Name: name}
}

func (g Greeter) Greet(name string) string {
    return name
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
    fn rust_function_body_macro_invocations_require_function_bodies() {
        let source = "fn checked() {\n    assert_eq!(1, 1);\n}\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();

        assert_eq!(ast.render(AstRenderOptions::default()), "fn checked");
        assert_eq!(
            ast.render(AstRenderOptions {
                include_function_bodies: true,
                ..AstRenderOptions::default()
            }),
            source.trim()
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
            "struct S {\n    a: bool,\n}\nenum E {\n    A,\n    B,\n}\nfn f1(a: String) -> bool {\n    !a.is_empty()\n}\nimpl S {\n    fn f2(&self) -> bool {\n        self.a\n    }\n}"
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
            "[1-4] struct S\n[5-9] enum E\n[10-13] fn f1\n[14-19] impl S\n> [15-18] fn f2"
        );
    }

    #[test]
    fn renders_edit_ready_locations_consistently_across_languages() {
        let options = AstRenderOptions {
            include_locations: true,
            ..AstRenderOptions::default()
        };
        let fixtures = [
            (AstLanguage::Rust, "fn run() {}\n", "[0-1] fn run"),
            (
                AstLanguage::Python,
                "def run():\n    pass\n",
                "[0-2] def run",
            ),
            (
                AstLanguage::JavaScript,
                "function run() {}\n",
                "[0-1] function run",
            ),
            (
                AstLanguage::TypeScript,
                "function run(): void {}\n",
                "[0-1] function run",
            ),
        ];

        for (language, source, expected) in fixtures {
            let ast = FileAst::parse(language, source).unwrap();
            assert_eq!(ast.render(options), expected);
        }
    }

    #[test]
    fn rust_doc_extraction_never_panics_on_valid_empty_or_doc_only_sources() {
        for source in ["", "\n", "\r\n", "//! docs\n", "/*! docs */\n"] {
            let rendered = std::panic::catch_unwind(|| {
                FileAst::parse(AstLanguage::Rust, source)
                    .unwrap()
                    .render(AstRenderOptions {
                        include_docs: true,
                        ..AstRenderOptions::default()
                    })
            });
            assert!(rendered.is_ok(), "panicked for {source:?}");
        }
    }

    #[test]
    fn rust_attributes_docs_and_edit_ready_locations_cover_the_owned_item() {
        let source = "before!();\n/// selected docs\n#[cfg(any(\n    unix,\n    windows,\n))]\n// rationale for the selected function\nfn selected() {\n    body!();\n}\nafter!();\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();
        let selected = ast
            .select_items(&AstSelector {
                item_patterns: vec!["selected".to_owned()],
                type_patterns: Vec::new(),
            })
            .unwrap();
        assert_eq!(selected[0].location.display(), "1-10");

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["selected".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            rendered,
            "/// selected docs\n#[cfg(any(\n    unix,\n    windows,\n))]\n// rationale for the selected function\n[1-10] fn selected() {\n           body!();\n       }"
        );
    }

    #[test]
    fn rust_docs_work_in_both_legal_attribute_orders_and_inline_modules() {
        let source = "#!/usr/bin/env rust-script\n// bootstrap comment\n#![cfg_attr(\n    all(),\n    allow(dead_code)\n)]\n//! crate docs\n\n/// before attribute\n#[derive(Debug)]\nstruct Before;\n\n#[derive(Debug)]\n/// after attribute\nstruct After;\n\n#[cfg(unix)]\nmod inner {\n    //! inner docs\n    fn child() {}\n}\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();
        assert_eq!(ast.root_docs.as_deref(), Some("//! crate docs"));
        let rendered = ast.render(AstRenderOptions {
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert!(rendered.contains("/// before attribute\nstruct Before"));
        assert!(rendered.contains("/// after attribute\nstruct After"));
        assert!(rendered.contains("mod inner\n> //! inner docs\n> fn child"));

        let signatures = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert!(signatures.contains("#[derive(Debug)]\n/// after attribute\nstruct After;"));
        assert!(signatures.contains("#[cfg(unix)]\nmod inner\n> //! inner docs\n> fn child()"));

        let bodies = ast.render(AstRenderOptions {
            include_type_bodies: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert!(bodies.contains("#[cfg(unix)]\nmod inner {\n    //! inner docs"));
        assert_eq!(bodies.matches("//! inner docs").count(), 1);
    }

    #[test]
    fn type_selection_handles_references_and_qualified_duplicate_names() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "struct Wrapper<T>(T);\ntrait Marker {}\nimpl<'a, T> Marker for &'a mut Wrapper<T> {}\nmod nested { struct Wrapper; impl Wrapper { fn nested() {} } }\nimpl crate::nested::Wrapper { fn external() {} }\n",
        )
        .unwrap();

        let bare = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Wrapper".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert!(bare.contains("impl Marker for &'a mut Wrapper<T>"));
        assert!(bare.contains("struct Wrapper"));

        let qualified = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["nested.Wrapper".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(
            qualified,
            "struct Wrapper\nimpl Wrapper\n> fn nested\nimpl crate::nested::Wrapper\n> fn external"
        );
    }

    #[test]
    fn rust_impl_type_paths_resolve_relative_to_their_module() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "struct Root;\nmod outer {\n    struct Local;\n    impl self::Local { fn local() {} }\n    mod nested {\n        struct Deep;\n        impl self::Deep { fn deep() {} }\n        impl super::Local { fn parent() {} }\n        impl super::super::Root { fn repeated_super() {} }\n        mod leaf { struct Child; }\n        impl leaf::Child { fn plain_nested() {} }\n        impl crate::Root { fn crate_root() {} }\n    }\n}\n",
        )
        .unwrap();

        let select = |pattern: &str| {
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec![pattern.to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap()
        };

        let local = select("outer.Local");
        assert!(local.contains("struct Local"));
        assert!(local.contains("impl self::Local"));
        assert!(local.contains("impl super::Local"));

        let deep = select("outer.nested.Deep");
        assert!(deep.contains("struct Deep"));
        assert!(deep.contains("impl self::Deep"));

        let child = select("outer.nested.leaf.Child");
        assert!(child.contains("struct Child"));
        assert!(child.contains("impl leaf::Child"));

        let root = select("Root");
        assert!(root.contains("struct Root"));
        assert!(root.contains("impl super::super::Root"));
        assert!(root.contains("impl crate::Root"));
    }

    #[test]
    fn qualified_rust_type_selectors_do_not_use_unresolved_impl_fallbacks() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "mod a { struct Type; }\nmod b { mod a { pub(crate) struct Type; } impl a::Type { fn nested_only() {} } }\n",
        )
        .unwrap();

        let top_level = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["a.Type".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(top_level, "struct Type");

        let nested = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["b.a.Type".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(nested, "struct Type\nimpl a::Type\n> fn nested_only");

        let bare = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Type".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(bare.matches("struct Type").count(), 2);
        assert!(bare.contains("impl a::Type"));
    }

    #[test]
    fn trait_type_bodies_include_associated_types() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "pub trait Service<T> where T: Clone {\n    type Output<'a>: Send where Self: 'a;\n    const VALUE: usize;\n    fn required(&self, value: T) -> bool;\n}\n",
        )
        .unwrap();

        let outline = ast.render(AstRenderOptions::default());
        assert!(outline.contains("> type Output"));
        let body = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Service".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert!(body.starts_with("pub trait Service<T> where T: Clone {"));
        assert!(body.contains("type Output<'a>: Send where Self: 'a;"));
    }

    #[test]
    fn rust_public_api_and_local_declarations_are_represented() {
        let source = "pub use crate::api::Thing;\n#[macro_export]\nmacro_rules! exported { () => {} }\nunsafe extern \"C\" {\n    pub safe fn foreign(value: i32) -> i32;\n    pub static FOREIGN: i32;\n}\nfn outer() {\n    fn local() {}\n    local_macro!();\n}\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();
        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            ..AstRenderOptions::default()
        });
        assert!(rendered.contains("pub use crate::api::Thing;"));
        assert!(rendered.contains("#[macro_export]\nmacro_rules! exported"));
        assert!(rendered.contains("unsafe extern \"C\""));
        assert!(rendered.contains("> pub safe fn foreign(value: i32) -> i32;"));
        assert!(rendered.contains("> pub static FOREIGN: i32;"));
        assert!(rendered.contains("> fn local()"));
        assert!(!rendered.contains("local_macro"));
    }

    #[test]
    fn ast_location_can_be_used_directly_as_an_apply_range() {
        let source = "fn before() {} // belongs to before\n// standalone rationale for selected\n/// selected docs\n#[inline]\n// keep this rationale with selected\nfn selected() {\n    work();\n}\nfn after() {}\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();
        let item = ast
            .select_items(&AstSelector {
                item_patterns: vec!["selected".to_owned()],
                type_patterns: Vec::new(),
            })
            .unwrap()
            .remove(0);
        assert_eq!(item.location.display(), "1-8");
        assert!(item.location.is_edit_ready);
        assert!(
            item.source_preamble
                .as_deref()
                .is_some_and(|preamble| preamble.starts_with("// standalone rationale"))
        );
        assert!(!item.source_preamble.as_deref().unwrap().contains("belongs"));
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "smartedit-ast-location-{}-{unique}.rs",
            process::id()
        ));
        fs::write(&path, source).unwrap();
        let program = EditProgram::from_modifications(vec![
            GenericModification::ReplaceRanges {
                target: FileRangeSelection::new(
                    &path,
                    RangeSet::single(
                        TextRange::new(item.location.start_line, item.location.end_line).unwrap(),
                    ),
                ),
                content: "fn replacement() {}\n".to_owned(),
                create_destination_if_missing: false,
                span: None,
            }
            .into(),
        ]);

        Executor::new().execute(&program).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "fn before() {} // belongs to before\nfn replacement() {}\nfn after() {}\n"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rust_detached_ordinary_comments_are_not_item_preambles() {
        let source = "// section banner\r\n\r\n/// selected docs\r\n#[inline]\r\nfn selected() {}\r\n\r\nmod nested {\r\n    // nested section\r\n\r\n    fn child() {}\r\n}\r\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();
        let selected = &ast.items[0];
        assert_eq!(selected.docs.as_deref(), Some("/// selected docs"));
        assert_eq!(selected.attributes.as_deref(), Some("#[inline]"));
        assert_eq!(
            selected.source_preamble.as_deref(),
            Some("/// selected docs\n#[inline]")
        );
        assert_eq!(selected.location.start_line, 2);

        let child = &ast.items[1].children[0];
        assert_eq!(child.name.as_deref(), Some("child"));
        assert_eq!(child.source_preamble, None);
        assert_eq!(child.attributes, None);
        assert_eq!(child.location.start_line, 9);
    }

    #[test]
    fn rust_macro_rules_items_use_source_syntax_in_summaries_and_signatures() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "#[macro_export]\nmacro_rules! exported { ($value:expr) => { $value } }\n",
        )
        .unwrap();
        let item = &ast.items[0];
        assert_eq!(item.summary, "macro_rules! exported");
        assert_eq!(item.signature.as_deref(), Some("macro_rules! exported"));
        assert_eq!(item.attributes.as_deref(), Some("#[macro_export]"));
        assert!(item.body.as_deref().unwrap().contains("($value:expr)"));
    }

    #[test]
    fn locations_annotate_items_that_share_source_lines() {
        let ast = FileAst::parse(
            AstLanguage::Rust,
            "fn first() {} fn second() {}\nmod inline { fn child() {} }\n",
        )
        .unwrap();

        assert_eq!(ast.items[0].location.display(), "0-1 shared-line");
        assert_eq!(ast.items[1].location.display(), "0-1 shared-line");
        assert!(!ast.items[0].location.is_edit_ready);
        assert!(!ast.items[1].location.is_edit_ready);
        assert_eq!(
            ast.items[2].children[0].location.display(),
            "1-2 shared-line"
        );
        assert!(!ast.items[2].children[0].location.is_edit_ready);

        assert_eq!(
            ast.render(AstRenderOptions {
                include_locations: true,
                ..AstRenderOptions::default()
            }),
            "[0-1 shared-line] fn first\n[0-1 shared-line] fn second\n[1-2] mod inline\n> [1-2 shared-line] fn child"
        );

        let javascript = FileAst::parse(
            AstLanguage::JavaScript,
            "const first = () => {}, second = () => {};\n",
        )
        .unwrap();
        assert_eq!(javascript.items.len(), 2);
        assert_eq!(javascript.items[0].location.display(), "0-1 shared-line");
        assert_eq!(javascript.items[1].location.display(), "0-1 shared-line");
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

    #[test]
    fn rust_doc_attributes_are_docs_and_preserve_owned_source_order() {
        let source = "#![doc = \"crate docs\"]\n#![doc(html_logo_url = \"logo.svg\")]\n#![allow(dead_code)]\n\n#[doc(hidden)]\n#[cfg(unix)]\n#[doc = \"item docs\"]\n#[doc(alias = \"documented\")]\n#[inline]\npub fn documented() {}\n";
        let ast = FileAst::parse(AstLanguage::Rust, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "#![doc = \"crate docs\"]\n#[doc = \"item docs\"]\nfn documented"
        );
        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "#![doc = \"crate docs\"]\n#[doc(hidden)]\n#[cfg(unix)]\n#[doc = \"item docs\"]\n#[doc(alias = \"documented\")]\n#[inline]\npub fn documented()"
        );
        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            }),
            "#[doc(hidden)]\n#[cfg(unix)]\n#[doc(alias = \"documented\")]\n#[inline]\npub fn documented()"
        );
        assert_eq!(ast.items[0].location.display(), "4-10");
    }

    #[test]
    fn parses_python_file_paths() {
        assert_eq!(
            AstLanguage::from_path(Path::new("example.py")),
            Some(AstLanguage::Python)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.pyi")),
            Some(AstLanguage::Python)
        );
    }

    #[test]
    fn parses_javascript_typescript_and_go_file_paths() {
        assert_eq!(
            AstLanguage::from_path(Path::new("example.js")),
            Some(AstLanguage::JavaScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.jsx")),
            Some(AstLanguage::JavaScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.ts")),
            Some(AstLanguage::TypeScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.d.ts")),
            Some(AstLanguage::TypeScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.mts")),
            Some(AstLanguage::TypeScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.cts")),
            Some(AstLanguage::TypeScript)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.tsx")),
            Some(AstLanguage::Tsx)
        );
        assert_eq!(
            AstLanguage::from_path(Path::new("example.go")),
            Some(AstLanguage::Go)
        );
    }

    #[test]
    fn renders_basic_go_outline() {
        let ast = FileAst::parse(AstLanguage::Go, GO_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "struct Greeter\n> field Name\ninterface Runner\n> fn Run\nconst DefaultName\nvar Count\nfunc NewGreeter\nmethod Greeter.Greet"
        );
    }

    #[test]
    fn renders_go_signatures_and_docs() {
        let ast = FileAst::parse(AstLanguage::Go, GO_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "// package docs\n// Greeter docs\ntype Greeter struct\n> Name string\ntype Runner interface\n> // Run docs\n> Run(task string) string\nconst DefaultName = \"world\"\nvar Count int\nfunc NewGreeter(name string) Greeter\nfunc (g Greeter) Greet(name string) string"
        );
    }

    #[test]
    fn selects_go_type_and_methods() {
        let ast = FileAst::parse(AstLanguage::Go, GO_SAMPLE).unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Greeter".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(
            rendered,
            "struct Greeter\n> field Name\nmethod Greeter.Greet"
        );
    }

    #[test]
    fn go_declaration_specs_have_independent_names_docs_and_locations() {
        let source = "package sample\n\ntype (\n    // Record docs\n    Record struct { Value int }\n    Alias = map[string]int\n)\n\nconst (\n    // paired values\n    First, Second = 1, 2\n    Third = 3\n)\nvar One, Two int\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "struct Record\n> field Value\ntype Alias\nconst First\nconst Second\nconst Third\nvar One\nvar Two"
        );
        let second = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Second".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            second,
            "// paired values\n[9-11 shared-line] const First, Second = 1, 2"
        );
        assert!(!ast.items[2].location.is_edit_ready);
        assert!(!ast.items[3].location.is_edit_ready);
        assert!(!ast.items[5].location.is_edit_ready);
        assert!(!ast.items[6].location.is_edit_ready);
        assert_eq!(ast.items[0].location.display(), "3-5");
        assert!(ast.items[0].location.is_edit_ready);

        let record = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Record".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            record,
            "// Record docs\n[3-5] type Record struct { Value int }"
        );

        let alias = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Alias".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(alias, "type Alias = map[string]int");
    }

    #[test]
    fn go_receiver_types_and_method_selectors_are_structural() {
        let source = "package sample\n\ntype Box[T any] struct { Value T }\nfunc (b *Box[T]) Pointer() {}\nfunc (Box[T]) Unnamed() {}\nfunc (b Box[T]) Value() {}\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();

        for (selector, expected) in [
            ("Box.Pointer", "method Box.Pointer"),
            ("Box.Unnamed", "method Box.Unnamed"),
            ("Unnamed", "method Box.Unnamed"),
        ] {
            assert_eq!(
                ast.render_with_selector(
                    &AstSelector {
                        item_patterns: vec![selector.to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions::default(),
                )
                .unwrap(),
                expected
            );
        }

        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Box".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "struct Box\n> field Value\nmethod Box.Pointer\nmethod Box.Unnamed\nmethod Box.Value"
        );
    }

    #[test]
    fn go_root_docs_are_anchored_to_the_package_clause() {
        let source = "//go:build linux\n// +build linux\n\n// Copyright Example\n\n// Package sample demonstrates docs.\npackage sample\n\n// Item docs.\nfunc Item() {}\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "//go:build linux\n// +build linux\n\n// Copyright Example\n\n// Package sample demonstrates docs.\n// Item docs.\nfunc Item"
        );
        assert_eq!(ast.items[0].location.display(), "8-10");
    }

    #[test]
    fn go_requires_a_complete_source_file_shape() {
        for (source, expected_line) in [
            ("", 0),
            ("func MissingPackage() {}\n", 0),
            ("const Before = 1\npackage sample\n", 0),
            ("package first\npackage second\n", 1),
            ("package sample\nvalue := 1\n", 1),
            ("package sample\nreturn\n", 1),
        ] {
            let ast = FileAst::parse(AstLanguage::Go, source).unwrap();
            assert!(
                ast.has_errors,
                "accepted invalid complete Go file: {source:?}"
            );
            assert_eq!(
                ast.first_error.map(|error| error.line),
                Some(expected_line),
                "wrong error location for {source:?}"
            );
        }

        let valid = FileAst::parse(
            AstLanguage::Go,
            "//go:build linux\n\n// Package sample docs.\npackage sample\n\nimport \"fmt\"\nvar Value = fmt.Sprint(1)\n",
        )
        .unwrap();
        assert!(!valid.has_errors);
        assert_eq!(valid.first_error, None);
    }

    #[test]
    fn go_grouped_var_specs_are_independent_and_group_context_renders_once() {
        let source = "package sample\n\n// Types group.\ntype (\n    TypeA int\n    TypeB = string\n)\n\n// Constants group.\nconst (\n    ConstantA = 1\n    ConstantB = 2\n)\n\n// Variables group.\nvar (\n    // Alpha docs.\n    Alpha int\n    Beta string\n)\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();
        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });

        for group_doc in [
            "// Types group.",
            "// Constants group.",
            "// Variables group.",
        ] {
            assert_eq!(rendered.matches(group_doc).count(), 1, "{rendered}");
        }
        assert!(rendered.contains("type TypeA int\ntype TypeB = string"));
        assert!(rendered.contains("const ConstantA = 1\nconst ConstantB = 2"));
        assert!(rendered.contains("// Alpha docs.\nvar Alpha int\nvar Beta string"));

        let beta = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Beta".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(beta, "[18-19] var Beta string");
        let beta_item = ast
            .select_items(&AstSelector {
                item_patterns: vec!["Beta".to_owned()],
                type_patterns: Vec::new(),
            })
            .unwrap()
            .remove(0);
        assert!(beta_item.location.is_edit_ready);
    }

    #[test]
    fn go_type_signatures_omit_bodies_while_type_bodies_are_complete() {
        let ast = FileAst::parse(
            AstLanguage::Go,
            "package sample\n\ntype Record[T any] struct {\n    Value T\n}\ntype Service interface {\n    Run(T) error\n}\n",
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            }),
            "type Record[T any] struct\n> Value T\ntype Service interface\n> Run(T) error"
        );
        assert_eq!(
            ast.render(AstRenderOptions {
                include_type_bodies: true,
                ..AstRenderOptions::default()
            }),
            "type Record[T any] struct {\n    Value T\n}\ntype Service interface {\n    Run(T) error\n}"
        );
    }

    #[test]
    fn go_functions_include_direct_local_declarations_only() {
        let source = "package sample\n\nfunc Work() {\n    const LocalA, LocalB = 1, 2\n    type Local = int\n    var LocalValue string\n    if true {\n        var Nested int\n    }\n}\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "func Work\n> const LocalA\n> const LocalB\n> type Local\n> var LocalValue"
        );
        assert!(!ast.render(AstRenderOptions::default()).contains("Nested"));
    }

    #[test]
    fn go_preambles_include_adjacent_directives_but_not_trailing_comments() {
        let source = "package sample\n\nvar prior int // belongs to prior\n//go:noinline\n// Work docs.\nfunc Work() {}\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();
        let work = &ast.items[1];

        assert_eq!(work.docs.as_deref(), Some("//go:noinline\n// Work docs."));
        assert_eq!(work.location.display(), "3-6");
        assert!(!work.docs.as_deref().unwrap().contains("belongs"));
        assert_eq!(
            ast.render(AstRenderOptions {
                include_function_bodies: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "var prior\n//go:noinline\n// Work docs.\nfunc Work() {}"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Work".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "//go:noinline\nfunc Work() {}"
        );
    }

    #[test]
    fn go_struct_and_interface_members_have_qualified_selectors() {
        let source = "package sample\n\ntype Record struct {\n    Left, Right int\n    embedded.Type\n}\ntype API interface {\n    // Call docs.\n    Call(string) error\n    Embedded\n    ~int | ~string\n}\n";
        let ast = FileAst::parse(AstLanguage::Go, source).unwrap();

        for (selector, expected) in [
            ("Record.Left", "field Left"),
            ("Record.Right", "field Right"),
            ("Record.embedded.Type", "field embedded.Type"),
            ("API.Call", "fn Call"),
            ("API.Embedded", "type Embedded"),
        ] {
            assert_eq!(
                ast.render_with_selector(
                    &AstSelector {
                        item_patterns: vec![selector.to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions::default(),
                )
                .unwrap(),
                expected
            );
        }
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["API.Call".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    include_docs: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "// Call docs.\nCall(string) error"
        );
    }

    #[test]
    fn renders_basic_python_outline() {
        let ast = FileAst::parse(AstLanguage::Python, PYTHON_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "class Greeter\n> def greet\n>> def normalize\nasync def run"
        );
    }

    #[test]
    fn renders_python_signatures_and_docs() {
        let ast = FileAst::parse(AstLanguage::Python, PYTHON_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "\"\"\"module docs\"\"\"\n\"\"\"class docs\"\"\"\nclass Greeter:\n> \"\"\"method docs\"\"\"\n> def greet(self, name: str) -> str:\n>> def normalize(value):\n@cached\nasync def run(task):"
        );
    }

    #[test]
    fn renders_python_type_and_function_bodies_when_requested() {
        let ast = FileAst::parse(AstLanguage::Python, PYTHON_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_type_bodies: true,
                include_function_bodies: true,
                include_docs: false,
                include_locations: false,
            }),
            "class Greeter:\n    \"\"\"class docs\"\"\"\n\n    def greet(self, name: str) -> str:\n        \"\"\"method docs\"\"\"\n\n        def normalize(value):\n            return value.strip()\n\n        return normalize(name)\n@cached\nasync def run(task):\n    return task()"
        );
    }

    #[test]
    fn selects_python_nested_items_by_glob_path() {
        let ast = FileAst::parse(AstLanguage::Python, PYTHON_SAMPLE).unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Greeter.greet.*".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "def normalize");
    }

    #[test]
    fn selects_python_type_and_methods() {
        let ast = FileAst::parse(AstLanguage::Python, PYTHON_SAMPLE).unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Greeter".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "class Greeter\n> def greet\n>> def normalize");
    }

    #[test]
    fn python_rendering_preserves_source_relative_indentation() {
        for source in [
            "def hanging(value: str,\n            fallback: str) -> str:\n    return value or fallback\n",
            "def hanging(value: str,\r\n            fallback: str) -> str:\r\n    return value or fallback\r\n",
        ] {
            let ast = FileAst::parse(AstLanguage::Python, source).unwrap();
            assert_eq!(
                ast.render(AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                }),
                "def hanging(value: str,\n            fallback: str) -> str:"
            );
            assert_eq!(
                ast.render(AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                }),
                "def hanging(value: str,\n            fallback: str) -> str:\n    return value or fallback"
            );
        }

        let nested = FileAst::parse(
            AstLanguage::Python,
            "class Outer:\n\t@decorator(\n\t\t\"arg\",\n\t)\n\tdef method(\n\t\tself,\n\t\tvalue,\n\t):\n\t\treturn value\n",
        )
        .unwrap();
        let rendered = nested
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Outer.method".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            rendered,
            "@decorator(\n\t\"arg\",\n)\ndef method(\n\tself,\n\tvalue,\n):\n\treturn value"
        );

        let one_line = FileAst::parse(
            AstLanguage::Python,
            "class Outer:\n    class Inner:\n        def method(self): return 1\n",
        )
        .unwrap();
        let rendered = one_line
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Outer.Inner".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(rendered, "class Inner:\n    def method(self): return 1");
    }

    #[test]
    fn python_signatures_end_at_the_structural_header_colon() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "@registered\nclass Outer:\n    # not part of the class signature\n    \"\"\"class docs\"\"\"\n\n    @traced\n    async def method(self) -> str:\n        # not part of the method signature\n        return \"ok\"\n",
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            }),
            "@registered\nclass Outer:\n> @traced\n> async def method(self) -> str:"
        );
    }

    #[test]
    fn python_docs_support_comments_parentheses_and_concatenation() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "#!/usr/bin/env python3\n# coding: utf-8\n(\"module \" \"docs\")\n\nclass Parenthesized:\n    # comments are not statements\n    (r\"\"\"class docs\"\"\")\n\n    def method(self):\n        # comments are not statements\n        (\"method \" \"docs\")\n        return None\n\nclass ByteDoc:\n    b\"not docs\"\n\nclass FStringDoc:\n    f\"not docs either\"\n",
        )
        .unwrap();

        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert_eq!(
            rendered,
            "\"module \" \"docs\"\nr\"\"\"class docs\"\"\"\nclass Parenthesized:\n> \"method \" \"docs\"\n> def method(self):\nclass ByteDoc:\nclass FStringDoc:"
        );
    }

    #[test]
    fn python_parenthesized_docs_skip_comments_at_every_nesting_level() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "(\n    # module wrapper comment\n    (\n        # nested module wrapper comment\n        \"module docs\"\n    )\n)\n\nclass Container:\n    (\n        # class wrapper comment\n        (\"class docs\")\n    )\n\n    def method(self):\n        (\n            # function wrapper comment\n            (\"method docs\")\n        )\n        return None\n\nclass TupleExpression:\n    (\"not docs\",)\n\nclass BinaryExpression:\n    (\"not \" + \"docs\")\n\ndef ConditionalExpression():\n    (\"not docs\" if True else \"still not docs\")\n",
        )
        .unwrap();
        assert!(!ast.has_errors);

        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert!(rendered.starts_with(
            "\"module docs\"\n\"class docs\"\nclass Container:\n> \"method docs\"\n> def method(self):"
        ));
        assert!(rendered.contains("class TupleExpression:"));
        assert!(rendered.contains("class BinaryExpression:"));
        assert!(rendered.contains("def ConditionalExpression():"));
        assert_eq!(rendered.matches("not docs").count(), 0);
        assert_eq!(rendered.matches("still not docs").count(), 0);
    }

    #[test]
    fn python_concatenated_docstrings_ignore_intervening_comments() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "(\n    \"module \"\n    # translator note\n    \"docs\"\n)\n\nclass Container:\n    (\n        \"class \"\n        # translator note\n        \"docs\"\n    )\n\n    def method(self):\n        (\n            \"method \"\n            # translator note\n            \"docs\"\n        )\n        return None\n\nclass NotDocs:\n    (\n        \"literal\"\n        # an operator still makes this non-documentation\n        + \"expression\"\n    )\n",
        )
        .unwrap();
        assert!(!ast.has_errors);

        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert!(rendered.starts_with(
            "\"module \"\n# translator note\n\"docs\"\n\"class \"\n# translator note\n\"docs\"\nclass Container:"
        ));
        assert!(
            rendered
                .contains("> \"method \"\n> # translator note\n> \"docs\"\n> def method(self):")
        );
        assert!(rendered.contains("class NotDocs:"));
        assert!(!rendered.contains("an operator still makes"));
    }

    #[test]
    fn python_template_strings_are_not_docstrings() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "t\"module template\"\n\nclass Container:\n    (T\"class \" \"template\")\n\n    def method(self):\n        (\n            t\"method \"\n            # concatenation bridge\n            \"template\"\n        )\n        return None\n",
        )
        .unwrap();
        assert!(!ast.has_errors);
        assert_eq!(ast.root_docs, None);
        assert_eq!(ast.items[0].docs, None);
        assert_eq!(ast.items[0].children[0].docs, None);

        let rendered = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        assert_eq!(rendered, "class Container:\n> def method(self):");
    }

    #[test]
    fn python_full_bodies_do_not_duplicate_docstrings() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "class Container:\n    \"\"\"class docs\"\"\"\n\n    def method(self):\n        \"\"\"method docs\"\"\"\n        return 1\n",
        )
        .unwrap();

        let class = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Container".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    include_function_bodies: true,
                    include_docs: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(class.matches("class docs").count(), 1);
        assert_eq!(class.matches("method docs").count(), 1);

        let method = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Container.method".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    include_docs: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(method.matches("method docs").count(), 1);
        assert!(method.starts_with("def method(self):"));
    }

    #[test]
    fn python_discovers_definitions_in_every_compound_suite() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            r#"def owner(value):
    if value == 1:
        def in_if(): pass
        class ClassInIf: pass
    elif value == 2:
        def in_elif(): pass
    else:
        def in_else(): pass
    for item in ():
        def in_for(): pass
    else:
        def in_for_else(): pass
    while value:
        def in_while(): pass
    try:
        def in_try(): pass
    except Exception:
        def in_except(): pass
    else:
        def in_try_else(): pass
    finally:
        def in_finally(): pass
    with context():
        @decorator
        def in_with(): pass
    match value:
        case 1:
            def in_case(): pass
"#,
        )
        .unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "def owner\n> def in_if\n> class ClassInIf\n> def in_elif\n> def in_else\n> def in_for\n> def in_for_else\n> def in_while\n> def in_try\n> def in_except\n> def in_try_else\n> def in_finally\n> def in_with\n> def in_case"
        );
        let selected = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["owner.in_case".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(selected, "def in_case():");
    }

    #[test]
    fn python_type_aliases_support_bodies_and_qualified_selection() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "type Alias[T] = tuple[T, ...]\n\nclass Namespace:\n    type Nested[\n        T,\n    ] = dict[str, T]\n",
        )
        .unwrap();
        assert!(!ast.has_errors);
        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "type Alias\nclass Namespace\n> type Nested"
        );

        let nested = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Namespace.Nested".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(nested, "type Nested[\n    T,\n] = dict[str, T]");

        let top_level = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Alias".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert_eq!(top_level, "type Alias[T] = tuple[T, ...]");

        let malformed = FileAst::parse(AstLanguage::Python, "type Broken[T] =\n").unwrap();
        assert!(malformed.has_errors);
    }

    #[test]
    fn python_qualified_type_selectors_distinguish_nested_classes() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "class First:\n    class Inner:\n        def first(self): pass\n\nclass Second:\n    class Inner:\n        def second(self): pass\n",
        )
        .unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["First.Inner".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(rendered, "class Inner\n> def first");
    }

    #[test]
    fn python_stub_declarations_render_like_python() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "from typing import Protocol, overload\n\nclass Service(Protocol):\n    value: str\n    def fetch(self, identifier: int) -> str: ...\n\n@overload\ndef helper(value: int) -> str: ...\n@overload\ndef helper(value: str) -> str: ...\n",
        )
        .unwrap();
        assert!(!ast.has_errors);
        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                ..AstRenderOptions::default()
            }),
            "class Service(Protocol):\n> def fetch(self, identifier: int) -> str:\n@overload\ndef helper(value: int) -> str:\n@overload\ndef helper(value: str) -> str:"
        );
        let service = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Service".to_owned()],
                },
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert!(service.contains("value: str"));
        assert!(service.contains("def fetch(self, identifier: int) -> str: ..."));
    }

    #[test]
    fn malformed_python_still_reports_parse_errors() {
        let ast = FileAst::parse(
            AstLanguage::Python,
            "def valid(): pass\ndef broken(: pass\nclass After: pass\n",
        )
        .unwrap();
        assert!(ast.has_errors);
        let error = ast.first_error.expect("syntax-error location");
        assert!(error.line >= 1);
    }

    #[test]
    fn renders_basic_javascript_outline() {
        let ast = FileAst::parse(AstLanguage::JavaScript, JAVASCRIPT_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "class Greeter\n> method greet\n>> function normalize\nasync function run"
        );
    }

    #[test]
    fn renders_javascript_signatures_and_docs() {
        let ast = FileAst::parse(AstLanguage::JavaScript, JAVASCRIPT_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** module docs */\n/** class docs */\nclass Greeter\n> /** method docs */\n> greet(name)\n>> function normalize(value)\nexport const run = async (task) =>"
        );
    }

    #[test]
    fn selects_javascript_nested_items_by_glob_path() {
        let ast = FileAst::parse(AstLanguage::JavaScript, JAVASCRIPT_SAMPLE).unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Greeter.greet.*".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(rendered, "function normalize");
    }

    #[test]
    fn javascript_docs_never_panic_on_valid_empty_comment_or_shebang_sources() {
        for source in [
            "",
            "\n",
            "\r\n",
            "   \n",
            "\n\n",
            "// banner\n",
            "/* banner */\n",
            "#!/usr/bin/env node\n",
            "// no final newline",
            "/* banner */ function inline_comment() {}\n",
        ] {
            let rendered = std::panic::catch_unwind(|| {
                FileAst::parse(AstLanguage::JavaScript, source)
                    .unwrap()
                    .render(AstRenderOptions {
                        include_signatures: true,
                        include_docs: true,
                        ..AstRenderOptions::default()
                    })
            });
            assert!(rendered.is_ok(), "panicked for {source:?}");
        }
    }

    #[test]
    fn javascript_comment_ownership_uses_comment_ranges_and_adjacency() {
        let source = r#"/** file docs */

/** first docs */
function first() {}
/* inline docs */ function inline_comment() {}
function second() {}
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** file docs */\n/** first docs */\nfunction first()\n/* inline docs */\nfunction inline_comment()\nfunction second()"
        );

        let adjacent = FileAst::parse(
            AstLanguage::JavaScript,
            "/** adjacent item docs */\nfunction documented() {}\n",
        )
        .unwrap();
        assert_eq!(adjacent.root_docs, None);
        assert_eq!(
            adjacent.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** adjacent item docs */\nfunction documented()"
        );
    }

    #[test]
    fn javascript_function_summaries_derive_modifiers_from_syntax_nodes() {
        let source = r#"function multiplies(value = left * right) {}
function async_parameter(async = 1) {}
const arrow_parameter = (async = 1) => {};
async function fetch_value() {}
function* generate() {}
async function* generate_async() {}
class Worker {
    static *generate() {}
    async *generateAsync() {}
    static async *generateBoth() {}
    static() {}
    async() {}
}
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "function multiplies\nfunction async_parameter\nfunction arrow_parameter\nasync function fetch_value\nfunction* generate\nasync function* generate_async\nclass Worker\n> static method* generate\n> async method* generateAsync\n> static async method* generateBoth\n> method static\n> method async"
        );
    }

    #[test]
    fn javascript_discovers_callable_class_fields_and_object_apis() {
        let source = r#"class Worker {
    handler = () => {};
    #privateHandler = function () {};
    static build = async () => {};
}

const api = {
    run() {},
    stop: () => {},
    nested: {
        ping: function* () {},
    },
};
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "class Worker\n> method handler\n> method #privateHandler\n> static async method build\nobject api\n> method run\n> function stop\n> object nested\n>> function* ping"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Worker.handler".to_owned(), "api.nested.*".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "method handler\nfunction* ping"
        );
    }

    #[test]
    fn javascript_anonymous_default_exports_use_a_stable_default_name() {
        let fixtures = [
            ("export default () => {};", "function default"),
            ("export default async () => {};", "async function default"),
            ("export default function () {};", "function default"),
            ("export default function* () {};", "function* default"),
            ("export default class {};", "class default"),
            ("export default () => <main />;", "function default"),
        ];

        for (source, expected) in fixtures {
            let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();
            assert!(!ast.has_errors, "unexpected parse error for {source:?}");
            assert_eq!(ast.render(AstRenderOptions::default()), expected);
            assert_eq!(
                ast.render_with_selector(
                    &AstSelector {
                        item_patterns: vec!["default".to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions::default(),
                )
                .unwrap(),
                expected
            );
        }

        let named =
            FileAst::parse(AstLanguage::JavaScript, "export default function App() {}").unwrap();
        assert_eq!(named.render(AstRenderOptions::default()), "function App");
    }

    #[test]
    fn javascript_parenthesized_values_are_dispatched_without_losing_outer_ownership() {
        let source = r#"const wrapped = (() => {});
const api = ({ run() {}, nested: ({ ping: (() => {}) }) });
assigned = ((function () {}));
(wrappedAssignment = (() => {}));
class Worker { handler = ((() => {})); }
export default ((() => {}));
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();
        assert!(!ast.has_errors);
        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "function wrapped\nobject api\n> method run\n> object nested\n>> function ping\nfunction assigned\nfunction wrappedAssignment\nclass Worker\n> method handler\nfunction default"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["default".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "[5-6] export default ((() => {}));"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Worker.handler".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "handler = ((() => {}));"
        );
    }

    #[test]
    fn javascript_member_assignments_preserve_qualified_paths() {
        let source = r#"module.exports = function () {};
Service.run = () => {};
Other.run = () => {};
Registry["start"] = () => {};
Service.prototype.stop = () => {};
this.handle = () => {};
class Owner {
    #private;
    setup() {
        this.#private = () => {};
    }
}
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "function module.exports\nfunction Service.run\nfunction Other.run\nfunction Registry.start\nfunction Service.prototype.stop\nfunction this.handle\nclass Owner\n> method setup\n>> function this.#private"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Service.run".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "function Service.run"
        );
    }

    #[test]
    fn javascript_multi_declarators_render_complete_declarations() {
        let source = r#"/** callables */
export const first = () => {}, ignored = 1,
    second = function () {};
"#;
        let ast = FileAst::parse(AstLanguage::JavaScript, source).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** callables */\nexport const first = () =>\nexport const second = function ()"
        );
        let complete_declaration =
            "export const first = () => {}, ignored = 1,\nsecond = function () {};";
        assert_eq!(
            ast.render(AstRenderOptions {
                include_function_bodies: true,
                ..AstRenderOptions::default()
            }),
            format!("{complete_declaration}\n{complete_declaration}")
        );
        assert_eq!(ast.items[0].location, ast.items[1].location);
    }

    #[test]
    fn javascript_and_typescript_callables_in_any_multi_declaration_are_not_edit_ready() {
        for (language, source) in [
            (
                AstLanguage::JavaScript,
                "const callable = () => {}, scalar = 1;\nconst other = function () {}, { value } = source;\nconst standalone = () => {};\n",
            ),
            (
                AstLanguage::TypeScript,
                "const callable = (): void => {}, scalar: number = 1;\nconst other = function (): void {}, { value } = source;\nconst standalone = (): void => {};\n",
            ),
        ] {
            let ast = FileAst::parse(language, source).unwrap();
            assert!(!ast.has_errors);
            assert_eq!(
                ast.items
                    .iter()
                    .map(|item| item.name.as_deref().unwrap())
                    .collect::<Vec<_>>(),
                ["callable", "other", "standalone"]
            );
            assert!(!ast.items[0].location.is_edit_ready);
            assert!(!ast.items[1].location.is_edit_ready);
            assert!(ast.items[2].location.is_edit_ready);
            assert!(ast.items[0].location.display().ends_with(" shared-line"));
            assert!(ast.items[1].location.display().ends_with(" shared-line"));
        }
    }

    #[test]
    fn malformed_javascript_reports_errors_and_shared_line_locations_are_annotated() {
        let malformed = FileAst::parse(
            AstLanguage::JavaScript,
            "function valid() {}\nfunction broken( {}\n",
        )
        .unwrap();
        assert!(malformed.has_errors);

        let ast = FileAst::parse(
            AstLanguage::JavaScript,
            "const first = () => {}, second = () => {};\n",
        )
        .unwrap();
        assert_eq!(
            ast.render(AstRenderOptions {
                include_locations: true,
                ..AstRenderOptions::default()
            }),
            "[0-1 shared-line] function first\n[0-1 shared-line] function second"
        );
    }

    #[test]
    fn renders_basic_typescript_outline() {
        let ast = FileAst::parse(AstLanguage::TypeScript, TYPESCRIPT_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "interface Greeter\n> method greet\nclass Service\n> method run\n>> function normalize\ntype Task\nenum Mode"
        );
    }

    #[test]
    fn renders_typescript_signatures_and_docs() {
        let ast = FileAst::parse(AstLanguage::TypeScript, TYPESCRIPT_SAMPLE).unwrap();

        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** module docs */\n/** interface docs */\nexport interface Greeter\n> /** method docs */\n> greet(name: string): string;\nexport class Service\n> run(task: string): string\n>> const normalize = (value: string) =>\nexport type Task = { id: string };\nexport enum Mode"
        );
    }

    #[test]
    fn selects_typescript_type_and_members() {
        let ast = FileAst::parse(AstLanguage::TypeScript, TYPESCRIPT_SAMPLE).unwrap();

        let rendered = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Service".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();

        assert_eq!(
            rendered,
            "class Service\n> method run\n>> function normalize"
        );
    }

    #[test]
    fn typescript_docs_are_range_safe_and_exclusively_owned() {
        for language in [AstLanguage::TypeScript, AstLanguage::Tsx] {
            for source in [
                "",
                "\n",
                " \t\r\n",
                "// banner\n",
                "/** banner */\n",
                "// banner\r\n",
                "/** banner */",
                "/* banner */ export interface Inline { value: string }\n",
            ] {
                let ast = FileAst::parse(language, source).unwrap();
                assert!(!ast.has_errors, "unexpected parse error for {source:?}");
                let _ = ast.render(AstRenderOptions {
                    include_docs: true,
                    ..AstRenderOptions::default()
                });
            }
        }

        let ast = FileAst::parse(
            AstLanguage::TypeScript,
            "/** API contract */\nexport interface API { run(): void }\n",
        )
        .unwrap();
        assert_eq!(
            ast.render(AstRenderOptions {
                include_signatures: true,
                include_docs: true,
                ..AstRenderOptions::default()
            }),
            "/** API contract */\nexport interface API\n> run(): void"
        );
    }

    #[test]
    fn typescript_ambient_declarations_are_complete_and_selectable() {
        let source = r#"/** declarations */
declare const version: string;
declare function parse(input: string): number;
declare abstract class Driver<T> {
    abstract connect(url: string): Promise<T>;
}
declare interface Legacy { old(): void }
declare type Id = string | number;
declare const enum AmbientMode { A, B }
declare namespace SDK {
    function create(): Driver<string>;
    namespace Inner { const name: string; }
}
declare module "virtual-package" { export function boot(): void; }
declare global { interface Window { injected: boolean } }
export declare function exported<T>(value: T): T;
"#;
        let ast = FileAst::parse(AstLanguage::TypeScript, source).unwrap();
        assert!(!ast.has_errors);
        assert_eq!(
            ast.render(AstRenderOptions::default()),
            "const version\nfunction parse\nabstract class Driver\n> abstract method connect\ninterface Legacy\n> method old\ntype Id\nconst enum AmbientMode\nnamespace SDK\n> function create\n> namespace Inner\n>> const name\nmodule virtual-package\n> function boot\nmodule global\n> interface Window\n>> property injected\nfunction exported"
        );

        let signatures = ast.render(AstRenderOptions {
            include_signatures: true,
            include_docs: true,
            ..AstRenderOptions::default()
        });
        for declaration in [
            "/** declarations */\ndeclare const version: string;",
            "declare function parse(input: string): number;",
            "declare abstract class Driver<T>",
            "declare namespace SDK",
            "declare module \"virtual-package\"",
            "declare global",
            "export declare function exported<T>(value: T): T;",
        ] {
            assert!(
                signatures.contains(declaration),
                "missing declaration {declaration:?} in {signatures}"
            );
        }
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["SDK.Inner.name".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "const name: string;"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["virtual-package.boot".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "function boot"
        );
    }

    #[test]
    fn typescript_declaration_exports_and_parameter_properties_are_represented() {
        let ast = FileAst::parse(
            AstLanguage::TypeScript,
            "export as namespace Widget;\nexport = Widget;\ndeclare namespace Widget { function create(): Widget; }\ndeclare module \"asset\" { const content: string; export default content; }\nclass Service { constructor(public readonly dependency: Dependency, plain: string, @inject private optional?: number) {} }\n",
        )
        .unwrap();
        assert!(!ast.has_errors);

        let signatures = ast.render(AstRenderOptions {
            include_signatures: true,
            ..AstRenderOptions::default()
        });
        for expected in [
            "export as namespace Widget;",
            "export = Widget;",
            "export default content;",
            "> public readonly dependency: Dependency",
            "> @inject private optional?: number",
        ] {
            assert!(
                signatures.contains(expected),
                "missing {expected:?} in {signatures}"
            );
        }
        let basic = ast.render(AstRenderOptions::default());
        assert!(basic.contains("> property dependency"));
        assert!(basic.contains("> property optional"));
        assert!(!basic.contains("property plain"));

        for (selector, expected) in [
            ("export-as-namespace", "export as namespace Widget;"),
            ("export=", "export = Widget;"),
        ] {
            assert_eq!(
                ast.render_with_selector(
                    &AstSelector {
                        item_patterns: vec![selector.to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions::default(),
                )
                .unwrap(),
                expected
            );
        }

        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Service.dependency".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "[4-5 shared-line] public readonly dependency: Dependency"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["asset.default".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "export default content;"
        );

        let declaration_file = FileAst::parse(
            AstLanguage::TypeScript,
            "declare class Ambient {\n    constructor(\n        public readonly x: number,\n        protected y?: string,\n        value?: boolean,\n    );\n}\n",
        )
        .unwrap();
        assert!(!declaration_file.has_errors);
        let ambient_basic = declaration_file.render(AstRenderOptions::default());
        assert!(ambient_basic.contains("> property x"));
        assert!(ambient_basic.contains("> property y"));
        assert!(!ambient_basic.contains("property value"));
        for (name, expected) in [
            ("x", "public readonly x: number"),
            ("y", "protected y?: string"),
        ] {
            assert_eq!(
                declaration_file
                    .render_with_selector(
                        &AstSelector {
                            item_patterns: vec![format!("Ambient.{name}")],
                            type_patterns: Vec::new(),
                        },
                        AstRenderOptions {
                            include_signatures: true,
                            ..AstRenderOptions::default()
                        },
                    )
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn typescript_export_bindings_are_recognized_from_syntax_across_trivia() {
        let source = r#"export as
/* namespace bridge */ namespace
Widget;
export /* assignment bridge */ =
Widget;
declare module "asset" {
    const content: string;
    export /* default bridge */ default
    content;
}
"#;
        let ast = FileAst::parse(AstLanguage::TypeScript, source).unwrap();
        assert!(!ast.has_errors);

        for (selector, fragments) in [
            ("export-as-namespace", &["namespace bridge", "Widget;"][..]),
            ("export=", &["assignment bridge", "=", "Widget;"][..]),
            ("asset.default", &["default bridge", "content;"][..]),
        ] {
            let rendered = ast
                .render_with_selector(
                    &AstSelector {
                        item_patterns: vec![selector.to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions {
                        include_signatures: true,
                        ..AstRenderOptions::default()
                    },
                )
                .unwrap();
            for fragment in fragments {
                assert!(
                    rendered.contains(fragment),
                    "missing {fragment:?} for {selector:?} in {rendered:?}"
                );
            }
        }
    }

    #[test]
    fn typescript_namespaces_overloads_and_abstract_summaries_are_structural() {
        let source = r#"namespace Simple { export function overloaded(value: string): string; }
namespace A.B { export const value = () => 1; }
module Legacy { export function start(): void; }
export namespace Outer { export namespace Inner { export const value = () => 1; } }
function overloaded(value: string): string;
function overloaded(value: number): number;
function overloaded(value: string | number) { return value; }
export abstract class ExportedAbstract { abstract run(): void; }
abstract class LocalAbstract { abstract stop(): void; }
const enum Mode { A }
"#;
        let ast = FileAst::parse(AstLanguage::TypeScript, source).unwrap();
        assert!(!ast.has_errors);
        let basic = ast.render(AstRenderOptions::default());
        for expected in [
            "namespace Simple\n> function overloaded",
            "namespace A.B\n> function value",
            "module Legacy\n> function start",
            "namespace Outer\n> namespace Inner\n>> function value",
            "abstract class ExportedAbstract\n> abstract method run",
            "abstract class LocalAbstract\n> abstract method stop",
            "const enum Mode",
        ] {
            assert!(basic.contains(expected), "missing {expected:?} in {basic}");
        }
        assert_eq!(basic.matches("function overloaded").count(), 4);
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["overloaded".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "function overloaded(value: string): string;\nfunction overloaded(value: number): number;\nfunction overloaded(value: string | number)"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["A.B.value".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions::default(),
            )
            .unwrap(),
            "function value"
        );
    }

    #[test]
    fn typescript_decorated_member_selection_owns_decorators_and_locations() {
        let source = r#"class Decorated {
    /** method docs */
    @first
    // between decorators
    @second
    // after final decorator
    method<T>(value: T): T { return value; }

    @field
    handler = () => 1;

    @observe
    get current(): number { return 1; }
}
"#;
        let ast = FileAst::parse(AstLanguage::TypeScript, source).unwrap();
        assert!(!ast.has_errors);
        let selector = AstSelector {
            item_patterns: vec!["Decorated.method".to_owned()],
            type_patterns: Vec::new(),
        };
        assert_eq!(
            ast.render_with_selector(
                &selector,
                AstRenderOptions {
                    include_signatures: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "/** method docs */\n[2-7] @first\n      // between decorators\n      @second\n      // after final decorator\n      method<T>(value: T): T"
        );
        assert_eq!(
            ast.render_with_selector(
                &selector,
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "@first\n// between decorators\n@second\n// after final decorator\nmethod<T>(value: T): T { return value; }"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Decorated.handler".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "@field\nhandler = () => 1;"
        );
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["Decorated.current".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_function_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "@observe\nget current(): number { return 1; }"
        );

        for selector in ["Decorated.handler", "Decorated.current"] {
            let rendered = ast
                .render_with_selector(
                    &AstSelector {
                        item_patterns: vec![selector.to_owned()],
                        type_patterns: Vec::new(),
                    },
                    AstRenderOptions {
                        include_signatures: true,
                        include_locations: true,
                        ..AstRenderOptions::default()
                    },
                )
                .unwrap();
            assert!(
                rendered.starts_with('['),
                "missing location in {rendered:?}"
            );
            assert!(rendered.contains('@'), "missing decorator in {rendered:?}");
        }
    }

    #[test]
    fn typescript_decorated_classes_own_inter_decorator_comments() {
        let source = r#"/** service docs */
@sealed
// decorator bridge
@registered()
// final decorator bridge
class Service {
    run(): void {}
}
"#;
        let ast = FileAst::parse(AstLanguage::TypeScript, source).unwrap();
        assert!(!ast.has_errors);
        let selector = AstSelector {
            item_patterns: Vec::new(),
            type_patterns: vec!["Service".to_owned()],
        };
        let signature = ast
            .render_with_selector(
                &selector,
                AstRenderOptions {
                    include_signatures: true,
                    include_docs: true,
                    include_locations: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert!(signature.starts_with("/** service docs */\n[1-8] @sealed"));
        for fragment in [
            "// decorator bridge",
            "@registered()",
            "// final decorator bridge",
            "class Service",
        ] {
            assert!(signature.contains(fragment), "{signature:?}");
        }

        let body = ast
            .render_with_selector(
                &selector,
                AstRenderOptions {
                    include_type_bodies: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap();
        assert!(body.starts_with(
            "@sealed\n// decorator bridge\n@registered()\n// final decorator bridge\nclass Service"
        ));
    }

    #[test]
    fn typescript_members_tsx_defaults_and_qualified_type_selectors_are_supported() {
        let source = r#"namespace One {
    export interface Config {
        readonly prop?: string;
        (value: string): number;
        new <T>(value: T): Config;
        [key: string]: unknown;
    }
}
namespace Two { export interface Config { other: boolean } }
class Service {
    public readonly size: number;
    private handler = () => <aside />;
    accessor value: string;
    [computed]: string;
    #secret: number;
    constructor(public readonly dependency: Dependency) {}
}
export default <T,>(props: { value: T }) => <main>{props.value}</main>;
"#;
        let ast = FileAst::parse(AstLanguage::Tsx, source).unwrap();
        assert!(!ast.has_errors);
        let basic = ast.render(AstRenderOptions::default());
        for expected in [
            "property prop",
            "call signature",
            "construct signature",
            "index signature",
            "field size",
            "method handler",
            "accessor value",
            "field [computed]",
            "field #secret",
            "method constructor",
            "function default",
        ] {
            assert!(basic.contains(expected), "missing {expected:?} in {basic}");
        }
        let one = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["One.Config".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert!(one.starts_with("interface Config\n> property prop"));
        assert!(!one.contains("other"));
        let qualified_glob = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["One.*".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert_eq!(qualified_glob, one);
        let bare = ast
            .render_with_selector(
                &AstSelector {
                    item_patterns: Vec::new(),
                    type_patterns: vec!["Config".to_owned()],
                },
                AstRenderOptions::default(),
            )
            .unwrap();
        assert!(bare.contains("property prop"));
        assert!(bare.contains("property other"));
        assert_eq!(
            ast.render_with_selector(
                &AstSelector {
                    item_patterns: vec!["default".to_owned()],
                    type_patterns: Vec::new(),
                },
                AstRenderOptions {
                    include_signatures: true,
                    ..AstRenderOptions::default()
                },
            )
            .unwrap(),
            "export default <T,>(props: { value: T }) =>"
        );
    }
}

fn collect_generic_items<'a>(
    node: Node<'a>,
    source: &str,
    parse_item: fn(Node<'a>, &str) -> Option<AstItem>,
) -> Vec<AstItem> {
    let mut cursor = node.walk();
    let mut items = Vec::new();
    for child in node.named_children(&mut cursor) {
        if let Some(item) = parse_item(child, source) {
            items.push(item);
        } else {
            items.extend(collect_generic_items(child, source, parse_item));
        }
    }
    items
}

fn parse_scala_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "scala",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "scala",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let mut items = collect_generic_items(root, source, parse_scala_item);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Scala,
        root_docs: None,
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_scala_item(node: Node<'_>, source: &str) -> Option<AstItem> {
    let (kind, kind_str) = match node.kind() {
        "class_definition" => (AstItemKind::Class, "class"),
        "object_definition" => (AstItemKind::Class, "object"),
        "trait_definition" => (AstItemKind::Trait, "trait"),
        "function_definition" => (AstItemKind::Function, "def"),
        _ => return None,
    };

    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let body = node.child_by_field_name("body");

    Some(AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: None,
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{} {}", kind_str, name),
        signature: Some(signature_text_with_body(node, body, source)),
        body: body.map(|b| trimmed_node_text(b, source)),
        children: body
            .map(|b| collect_generic_items(b, source, parse_scala_item))
            .unwrap_or_default(),
    })
}

fn parse_java_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "java",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "java",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let mut items = collect_generic_items(root, source, parse_java_item);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Java,
        root_docs: None,
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_java_item(node: Node<'_>, source: &str) -> Option<AstItem> {
    let (kind, kind_str) = match node.kind() {
        "class_declaration" | "record_declaration" => (AstItemKind::Class, "class"),
        "interface_declaration" | "annotation_type_declaration" => {
            (AstItemKind::Interface, "interface")
        }
        "enum_declaration" => (AstItemKind::Enum, "enum"),
        "method_declaration" | "constructor_declaration" => (AstItemKind::Function, "method"),
        _ => return None,
    };

    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());
    let body = node.child_by_field_name("body");

    Some(AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: None,
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{} {}", kind_str, name),
        signature: Some(signature_text_with_body(node, body, source)),
        body: body.map(|b| trimmed_node_text(b, source)),
        children: body
            .map(|b| collect_generic_items(b, source, parse_java_item))
            .unwrap_or_default(),
    })
}

fn parse_kotlin_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "kotlin",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "kotlin",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let mut items = collect_generic_items(root, source, parse_kotlin_item);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Kotlin,
        root_docs: None,
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_kotlin_item(node: Node<'_>, source: &str) -> Option<AstItem> {
    let (kind, kind_str) = match node.kind() {
        "class_declaration" => (AstItemKind::Class, "class"),
        "object_declaration" => (AstItemKind::Class, "object"),
        "function_declaration" => (AstItemKind::Function, "fun"),
        _ => return None,
    };

    let name = child_text_by_field(node, "identifier", source)
        .or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "type_identifier" || c.kind() == "simple_identifier")
                .map(|c| trimmed_node_text(c, source))
        })
        .unwrap_or_else(|| "<anonymous>".to_owned());

    let body = node.child_by_field_name("body");

    Some(AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: None,
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{} {}", kind_str, name),
        signature: Some(signature_text_with_body(node, body, source)),
        body: body.map(|b| trimmed_node_text(b, source)),
        children: body
            .map(|b| collect_generic_items(b, source, parse_kotlin_item))
            .unwrap_or_default(),
    })
}

fn parse_lua_ast(source: &str) -> Result<FileAst> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|message| SmartEditError::AstParseSetupFailed {
            language: "lua",
            message: message.to_string(),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| SmartEditError::AstParseFailed {
            language: "lua",
            message: "tree-sitter returned no parse tree".to_owned(),
        })?;
    let root = tree.root_node();
    let mut items = collect_generic_items(root, source, parse_lua_item);
    mark_overlapping_sibling_locations(&mut items);

    Ok(FileAst {
        language: AstLanguage::Lua,
        root_docs: None,
        items,
        has_errors: root.has_error(),
        first_error: first_syntax_error(root),
    })
}

fn parse_lua_item(node: Node<'_>, source: &str) -> Option<AstItem> {
    let (kind, kind_str) = match node.kind() {
        "function_declaration" | "local_function_declaration" => {
            (AstItemKind::Function, "function")
        }
        _ => return None,
    };

    let name =
        child_text_by_field(node, "name", source).unwrap_or_else(|| "<anonymous>".to_owned());

    Some(AstItem {
        kind,
        name: Some(name.clone()),
        associated_type: None,
        location: location_for_node(node, source),
        docs: None,
        inner_docs: None,
        attributes: None,
        source_preamble: None,
        summary: format!("{} {}", kind_str, name),
        signature: Some(trimmed_node_text(node, source)),
        body: None,
        children: Vec::new(),
    })
}
