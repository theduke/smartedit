use std::fs;

#[test]
fn test_print_sexp() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .unwrap();
    let source = fs::read_to_string("tests/fixtures/test.php").unwrap();
    let tree = parser.parse(&source, None).unwrap();
    println!("=== php ===");
    println!("{}", tree.root_node().to_sexp());

    parser
        .set_language(&tree_sitter_toml_ng::LANGUAGE.into())
        .unwrap();
    let source = fs::read_to_string("tests/fixtures/test.toml").unwrap();
    let tree = parser.parse(&source, None).unwrap();
    println!("=== toml ===");
    println!("{}", tree.root_node().to_sexp());
}
