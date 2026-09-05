use smartedit::{AstItemKind, AstLanguage, AstSelector, FileAst};

#[test]
fn cpp_conversion_operators_keep_type_names_without_parameters() {
    let source = "class Box {\n  operator bool() const;\n  operator int() const { return 1; }\n  operator const int*() const;\n  operator int&();\n};\nBox::operator bool() const { return true; }\n";
    let ast = FileAst::parse(AstLanguage::Cpp, source).unwrap();
    assert!(!ast.has_errors);
    for name in [
        "operator bool",
        "operator int",
        "operator const int*",
        "operator int&",
    ] {
        let item = ast.items[0]
            .children
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(item.kind, AstItemKind::Function);
    }
    assert_eq!(ast.items[1].name.as_deref(), Some("Box.operator bool"));
    assert_eq!(
        ast.items[1].signature.as_deref(),
        Some("Box::operator bool() const")
    );
    assert_eq!(
        ast.items[1].body.as_deref(),
        Some("Box::operator bool() const { return true; }")
    );
    let selected = ast
        .select_items(&AstSelector {
            item_patterns: vec!["Box.operator int".to_owned()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(selected.len(), 1);
}

#[test]
fn cpp_member_pointer_recovery_keeps_the_field_identifier() {
    let ast = FileAst::parse(
        AstLanguage::Cpp,
        "class Box { void (Box::*callback)(int); };",
    )
    .unwrap();
    // The bundled grammar reports Box:: as ERROR inside the parenthesized
    // declarator. Retain the recovered pointer identifier without treating it
    // as a function or mistaking the error's scope identifier for its name.
    assert!(ast.has_errors);
    let selected = ast
        .select_items(&AstSelector {
            item_patterns: vec!["Box.callback".to_owned()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].kind, AstItemKind::Field);
}
