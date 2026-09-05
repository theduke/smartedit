use smartedit::{AstRenderOptions, AstSelector, FileAst, parse_file_ast};
use std::path::Path;

fn parse(extension: &str, source: &str) -> FileAst {
    let ast = parse_file_ast(Path::new(&format!("example.{extension}")), source).unwrap();
    assert!(!ast.has_errors, "unexpected syntax errors in {source}");
    ast
}

fn selected_body(ast: &FileAst, name: &str) -> String {
    ast.render_with_selector(
        &AstSelector {
            item_patterns: vec![name.to_owned()],
            ..Default::default()
        },
        AstRenderOptions {
            include_function_bodies: true,
            include_type_bodies: true,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn yaml_flow_pairs_have_the_same_outline_and_selectors_as_block_pairs() {
    for source in [
        "name: example\nmetadata:\n  version: 1\n",
        "{name: example, metadata: {version: 1}}\n",
        "name: example\nmetadata: {version: 1}\n",
    ] {
        let ast = parse("yaml", source);
        assert_eq!(
            ast.render(Default::default()),
            "key name\nkey metadata\n> key version"
        );
        assert_eq!(selected_body(&ast, "metadata.version"), "version: 1");
        assert_eq!(selected_body(&ast, "name"), "name: example");
    }
}

#[test]
fn json_bodies_preserve_keys_and_values_without_duplicating_children() {
    let ast = parse("json", r#"{"metadata": {"version": 1}}"#);
    assert_eq!(
        ast.render(Default::default()),
        "key \"metadata\"\n> key \"version\""
    );
    assert_eq!(
        selected_body(&ast, "\"metadata\""),
        "\"metadata\": {\"version\": 1}"
    );
    assert_eq!(
        selected_body(&ast, "\"metadata\".\"version\""),
        "\"version\": 1"
    );
}

#[test]
fn toml_quoted_tables_and_keys_have_normalized_selector_paths() {
    let source = "[\"my package\"]\n\"my key\" = 1\n'other key' = 2\n";
    let ast = parse("toml", source);
    assert_eq!(
        ast.render(Default::default()),
        "table my package\n> key my key\n> key other key"
    );
    assert_eq!(selected_body(&ast, "my package.my key"), "\"my key\" = 1");
    assert_eq!(
        selected_body(&ast, "my package.other key"),
        "'other key' = 2"
    );
    assert_eq!(selected_body(&ast, "my package"), source.trim_end());
}

#[test]
fn toml_dotted_keys_normalize_whitespace_and_each_quoted_component() {
    let source = "[ server . \"my package\" ]\n\"my key\" . value = 1\n";
    let ast = parse("toml", source);
    assert_eq!(
        ast.render(Default::default()),
        "table server.my package\n> key my key.value"
    );
    assert_eq!(
        selected_body(&ast, "server.my package.my key.value"),
        "\"my key\" . value = 1"
    );
}

#[test]
fn toml_basic_key_escapes_are_decoded_but_literal_keys_are_preserved() {
    let source = "[\"pack\\u0061ge\"]\n\"n\\U00000061me\" = 1\n'literal\\key' = 2\n";
    let ast = parse("toml", source);
    assert_eq!(ast.items[0].name.as_deref(), Some("package"));
    assert_eq!(ast.items[0].children[0].name.as_deref(), Some("name"));
    assert_eq!(
        ast.items[0].children[1].name.as_deref(),
        Some("literal\\key")
    );
    assert_eq!(
        selected_body(&ast, "package.name"),
        "\"n\\U00000061me\" = 1"
    );
}

#[test]
fn toml_type_bodies_include_table_contents_and_array_table_headers() {
    for source in [
        "[package]\nname = 'example'\n",
        "[[package]]\nname = 'example'\n",
    ] {
        let ast = parse("toml", source);
        assert_eq!(
            ast.render(AstRenderOptions {
                include_type_bodies: true,
                ..Default::default()
            }),
            source.trim_end()
        );
    }
}
