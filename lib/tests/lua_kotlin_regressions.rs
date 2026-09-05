use smartedit::{AstItemKind, AstRenderOptions, AstSelector, parse_file_ast};
use std::path::Path;

#[test]
fn lua_renders_headers_and_complete_bodies_and_selects_nested_functions() {
    let source = "local function outer(x)\n  local function inner(y)\n    return y + 1\n  end\n  return inner(x)\nend";
    let ast = parse_file_ast(Path::new("nested.lua"), source).unwrap();
    assert!(!ast.has_errors);
    assert_eq!(
        ast.render(AstRenderOptions::default()),
        "function outer\n> function inner"
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_signatures: true,
            ..Default::default()
        }),
        "local function outer(x)\n> local function inner(y)"
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_function_bodies: true,
            ..Default::default()
        }),
        source
    );
    let selected = ast
        .select_items(&AstSelector {
            item_patterns: vec!["outer.inner".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].signature.as_deref(),
        Some("local function inner(y)")
    );
    assert_eq!(
        selected[0].body.as_deref(),
        Some("local function inner(y)\n    return y + 1\n  end")
    );
}

#[test]
fn lua_empty_functions_have_header_only_signatures_and_complete_bodies() {
    let source = "function empty() end";
    let ast = parse_file_ast(Path::new("empty.lua"), source).unwrap();
    assert!(!ast.has_errors);
    assert_eq!(
        ast.render(AstRenderOptions {
            include_signatures: true,
            ..Default::default()
        }),
        "function empty()"
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_function_bodies: true,
            ..Default::default()
        }),
        source
    );
}

#[test]
fn kotlin_enum_preserves_members_and_complete_source() {
    let source = "enum class Color {\n    RED, BLUE;\n    fun label(): String = name\n}";
    let ast = parse_file_ast(Path::new("enum.kt"), source).unwrap();
    assert!(!ast.has_errors);
    assert_eq!(ast.items[0].kind, AstItemKind::Enum);
    assert_eq!(
        ast.render(AstRenderOptions::default()),
        "enum Color\n> fun label"
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_signatures: true,
            ..Default::default()
        }),
        "enum class Color\n> fun label(): String"
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_type_bodies: true,
            ..Default::default()
        }),
        source
    );
    let selected = ast
        .render_with_selector(
            &AstSelector {
                item_patterns: vec!["Color.label".into()],
                ..Default::default()
            },
            AstRenderOptions {
                include_function_bodies: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(selected, "fun label(): String = name");
}

#[test]
fn kotlin_bodyless_declarations_remain_visible_with_type_bodies() {
    let source = "class Empty";
    let ast = parse_file_ast(Path::new("empty.kt"), source).unwrap();
    assert!(!ast.has_errors);
    assert_eq!(
        ast.render(AstRenderOptions {
            include_type_bodies: true,
            ..Default::default()
        }),
        source
    );
}
