use smartedit::{AstRenderOptions, parse_file_ast};
use std::fs;
use std::path::Path;

#[test]
fn test_java_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.java").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.java"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("JAVA:\n{}", rendered);
    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("method myMethod"));
    assert!(rendered.contains("interface MyInterface"));
    assert!(rendered.contains("method doSomething"));
    assert!(rendered.contains("enum MyEnum"));
    assert!(rendered.contains("class MyRecord"));
    assert!(rendered.contains("interface MyAnnotation"));
    assert!(rendered.contains("method value"));
}

#[test]
fn test_kotlin_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.kt").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.kt"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("KOTLIN:\n{}", rendered);
    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("fun myMethod"));
    assert!(rendered.contains("object MyObject"));
    assert!(rendered.contains("fun doSomething"));
    assert!(rendered.contains("fun topLevelFunction"));
}

#[test]
fn test_scala_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.scala").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.scala"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("SCALA:\n{}", rendered);
    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("def myMethod"));
    assert!(rendered.contains("object MyObject"));
    assert!(rendered.contains("def doSomething"));
    assert!(rendered.contains("trait MyTrait"));
    assert!(rendered.contains("def abstractMethod"));
}

#[test]
fn test_lua_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.lua").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.lua"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("function top_level_function"));
    assert!(rendered.contains("function local_function"));
}

#[test]
fn test_rust_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.rs").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.rs"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("struct MyStruct"));
    assert!(rendered.contains("fn new"));
    assert!(rendered.contains("fn my_method"));
    assert!(rendered.contains("trait MyTrait"));
    assert!(rendered.contains("fn abstract_method"));
    assert!(rendered.contains("enum MyEnum"));
}

#[test]
fn test_go_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.go").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.go"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("GO:\n{}", rendered);
    assert!(rendered.contains("struct MyStruct"));
    assert!(rendered.contains("func NewMyStruct"));
    assert!(rendered.contains("method MyStruct.MyMethod"));
    assert!(rendered.contains("interface MyInterface"));
}

#[test]
fn test_python_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.py").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.py"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("def __init__"));
    assert!(rendered.contains("def my_method"));
    assert!(rendered.contains("def top_level_function"));
}

#[test]
fn test_javascript_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.js").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.js"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("method constructor"));
    assert!(rendered.contains("method myMethod"));
    assert!(rendered.contains("function topLevelFunction"));
}

#[test]
fn test_typescript_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.ts").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.ts"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("interface MyInterface"));
    assert!(rendered.contains("method abstractMethod"));
    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("method constructor"));
    assert!(rendered.contains("method myMethod"));
    assert!(rendered.contains("function topLevelFunction"));
    assert!(rendered.contains("type MyType"));
}

#[test]
fn test_tsx_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.tsx").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.tsx"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("interface MyProps"));
    assert!(rendered.contains("function MyComponent"));
}

#[test]
fn test_c_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.c").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.c"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("struct MyStruct"));
    assert!(rendered.contains("enum MyEnum"));
    assert!(rendered.contains("function my_function"));
}

#[test]
fn test_cpp_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.cpp").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.cpp"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("class/struct MyClass"));
    assert!(rendered.contains("enum MyEnum"));
    assert!(rendered.contains("function my_function"));
}

#[test]
fn test_csharp_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.cs").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.cs"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("class/struct MyClass"));
    assert!(rendered.contains("method myMethod"));
    assert!(rendered.contains("interface MyInterface"));
    assert!(rendered.contains("method doSomething"));
    assert!(rendered.contains("enum MyEnum"));
    assert!(rendered.contains("class/struct MyRecord"));
}

#[test]
fn test_ruby_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.rb").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.rb"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("module MyModule"));
    assert!(rendered.contains("class MyClass"));
    assert!(rendered.contains("def my_method"));
    assert!(rendered.contains("def class_method"));
}

#[test]
fn test_php_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.php").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.php"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("PHP:\n{}", rendered);
    assert!(rendered.contains("class/trait MyClass"));
    assert!(rendered.contains("function/method myMethod"));
    assert!(rendered.contains("interface MyInterface"));
    assert!(rendered.contains("function/method doSomething"));
    assert!(rendered.contains("class/trait MyTrait"));
    assert!(rendered.contains("function/method myTraitMethod"));
    assert!(rendered.contains("enum MyEnum"));
    assert!(rendered.contains("function/method topLevelFunction"));
}

#[test]
fn test_bash_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.sh").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.sh"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("function my_function"));
    assert!(rendered.contains("function my_other_function"));
}

#[test]
fn test_json_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.json").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.json"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("key \"name\""));
    assert!(rendered.contains("key \"version\""));
    assert!(rendered.contains("key \"dependencies\""));
}

#[test]
fn test_yaml_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.yaml").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.yaml"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());

    assert!(rendered.contains("key name"));
    assert!(rendered.contains("key version"));
    assert!(rendered.contains("key dependencies"));
}

#[test]
fn test_toml_ast_parsing() {
    let source = fs::read_to_string("tests/fixtures/test.toml").unwrap();
    let ast = parse_file_ast(Path::new("tests/fixtures/test.toml"), &source).unwrap();
    let rendered = ast.render(AstRenderOptions::default());
    println!("TOML:\n{}", rendered);
    assert!(rendered.contains("table package"));
    assert!(rendered.contains("key name"));
    assert!(rendered.contains("key version"));
    assert!(rendered.contains("table dependencies"));
    assert!(rendered.contains("key foo"));
}
