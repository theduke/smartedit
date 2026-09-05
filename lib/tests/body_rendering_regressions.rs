use smartedit::{AstRenderOptions, AstSelector, parse_file_ast};
use std::path::Path;

fn assert_full_declaration(path: &str, source: &str, selector: &str, expected: &str) {
    let ast = parse_file_ast(Path::new(path), source).unwrap();
    assert!(!ast.has_errors, "invalid fixture: {path}: {source}");
    let rendered = ast
        .render_with_selector(
            &AstSelector {
                item_patterns: vec![selector.to_owned()],
                type_patterns: Vec::new(),
            },
            AstRenderOptions {
                include_type_bodies: true,
                include_function_bodies: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(rendered, expected, "{path}: {selector}");
}

#[test]
fn java_bodies_preserve_class_and_method_declarations() {
    let source =
        "abstract class Box {\n    abstract int value();\n    int empty() { return 1; }\n}";
    assert_full_declaration("test.java", source, "Box", source);
    assert_full_declaration("test.java", source, "Box.value", "abstract int value();");
    assert_full_declaration(
        "test.java",
        source,
        "Box.empty",
        "int empty() { return 1; }",
    );
}

#[test]
fn scala_bodies_preserve_expression_and_abstract_declarations() {
    let source = "trait Box {\n  def value: Int\n  def add(x: Int): Int = x + 1\n}";
    assert_full_declaration("test.scala", source, "Box", source);
    assert_full_declaration("test.scala", source, "Box.value", "def value: Int");
    assert_full_declaration(
        "test.scala",
        source,
        "Box.add",
        "def add(x: Int): Int = x + 1",
    );
}

#[test]
fn ruby_bodies_preserve_method_headers_and_endings() {
    let source = "class Box\n  def add(x)\n    x + 1\n  end\n  def empty\n  end\nend";
    assert_full_declaration("test.rb", source, "Box", source);
    assert_full_declaration("test.rb", source, "Box.add", "def add(x)\n  x + 1\nend");
    assert_full_declaration("test.rb", source, "Box.empty", "def empty\nend");
}

#[test]
fn php_bodies_preserve_class_and_abstract_method_declarations() {
    let declaration = "abstract class Box {\n  abstract public function value(): int;\n}";
    let source = format!("<?php\n{declaration}");
    assert_full_declaration("test.php", &source, "Box", declaration);
    assert_full_declaration(
        "test.php",
        &source,
        "Box.value",
        "abstract public function value(): int;",
    );
    assert_full_declaration(
        "test.php",
        "<?php function add($x) { return $x + 1; }",
        "add",
        "function add($x) { return $x + 1; }",
    );
}

#[test]
fn bash_bodies_preserve_function_declarations() {
    let source = "greet() {\n  echo hello\n}";
    assert_full_declaration("test.sh", source, "greet", source);
}
