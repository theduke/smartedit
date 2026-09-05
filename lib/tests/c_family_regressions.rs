use smartedit::{
    AstItem, AstItemKind, AstLanguage, AstRenderOptions, AstSelector, EditProgram, Executor,
    FileAst, FileRangeSelection, GenericModification, RangeSet, TextRange,
};

fn parse(language: AstLanguage, source: &str) -> FileAst {
    let ast = FileAst::parse(language, source).unwrap();
    assert!(
        !ast.has_errors,
        "unexpected syntax error: {:?}",
        ast.first_error
    );
    ast
}

fn select(ast: &FileAst, path: &str) -> AstItem {
    let items = ast
        .select_items(&AstSelector {
            item_patterns: vec![path.to_owned()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(items.len(), 1, "{path}");
    items.into_iter().next().unwrap()
}

#[test]
fn c_declarator_names_distinguish_functions_from_function_pointers() {
    let source = "int add(int a, int b) { return a + b; }\nint *allocate(void);\nint (*callback)(int);\nint (*callbacks[2])(int);\nint (*factory(void))(int);\n";
    let ast = parse(AstLanguage::C, source);
    for name in ["add", "allocate", "factory"] {
        assert_eq!(select(&ast, name).kind, AstItemKind::Function, "{name}");
    }
    for name in ["callback", "callbacks"] {
        assert_eq!(select(&ast, name).kind, AstItemKind::Field, "{name}");
    }
    assert_eq!(
        select(&ast, "add").signature.as_deref(),
        Some("int add(int a, int b)")
    );
}

#[test]
fn declarations_keep_all_names_and_embedded_types_without_claiming_independent_ranges() {
    for language in [AstLanguage::C, AstLanguage::Cpp] {
        let source = "int count = 1;\nstruct Point {\n    int x;\n} point, other;\nint first(void), second(void);\ntypedef unsigned long Size, OtherSize;\n";
        let ast = parse(language, source);
        assert_eq!(select(&ast, "count").kind, AstItemKind::Field);
        assert_eq!(select(&ast, "Point.x").kind, AstItemKind::Field);
        for name in ["point", "other", "first", "second", "Size", "OtherSize"] {
            assert!(!select(&ast, name).location.is_edit_ready, "{name}");
        }
        assert_eq!(select(&ast, "second").kind, AstItemKind::Function);
        assert_eq!(select(&ast, "OtherSize").kind, AstItemKind::TypeAlias);
        let types = ast
            .select_items(&AstSelector {
                type_patterns: vec!["Point".to_owned()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name.as_deref(), Some("Point"));
    }
}

#[test]
fn cpp_method_prototypes_are_selectable_and_field_pointers_are_not_methods() {
    let source = "class Box {\npublic:\n    int get() const;\n    virtual void run() = 0;\n    int (*callback)(int);\n    int &value();\n};\n";
    let ast = parse(AstLanguage::Cpp, source);
    for name in ["Box.get", "Box.run", "Box.value"] {
        assert_eq!(select(&ast, name).kind, AstItemKind::Function);
    }
    assert_eq!(select(&ast, "Box.callback").kind, AstItemKind::Field);
    assert_eq!(
        select(&ast, "Box.get").signature.as_deref(),
        Some("int get() const;")
    );
    assert_eq!(
        select(&ast, "Box.run").body.as_deref(),
        Some("virtual void run() = 0;")
    );
}

#[test]
fn template_ranges_own_the_wrapper_and_preserve_the_following_declaration() {
    let source = "template<typename T>\nT identity(T x) {\n    return x;\n}\nvoid unrelated() {}\n";
    let ast = parse(AstLanguage::Cpp, source);
    let function = select(&ast, "identity");
    assert_eq!(function.location.display(), "0-4");
    assert_eq!(
        function.signature.as_deref(),
        Some("template<typename T>\nT identity(T x)")
    );
    assert_eq!(
        function.body.as_deref(),
        Some(
            source
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
                .as_str()
        )
    );
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "smartedit-template-{}-{unique}.cpp",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let program = EditProgram::from_modifications(vec![
        GenericModification::DeleteRanges {
            target: FileRangeSelection::new(
                &path,
                RangeSet::single(
                    TextRange::new(function.location.start_line, function.location.end_line)
                        .unwrap(),
                ),
            ),
            span: None,
        }
        .into(),
    ]);
    Executor::new().execute(&program).unwrap();
    let remaining = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(remaining, "void unrelated() {}\n");
    assert_eq!(parse(AstLanguage::Cpp, &remaining).items.len(), 1);
}

#[test]
fn template_type_bodies_and_locations_include_the_semicolon() {
    let source = "template<class T>\nclass Box {\n    T get() const;\n};\n";
    let ast = parse(AstLanguage::Cpp, source);
    let class = select(&ast, "Box");
    assert_eq!(class.location.display(), "0-4");
    assert_eq!(
        class.signature.as_deref(),
        Some("template<class T>\nclass Box")
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_type_bodies: true,
            ..Default::default()
        }),
        source.trim_end()
    );
    assert_eq!(
        select(&ast, "Box.get").signature.as_deref(),
        Some("T get() const;")
    );
}

#[test]
fn c_and_cpp_full_function_bodies_retain_the_signature() {
    let source = "int add(int a, int b) {\n    return a + b;\n}\n";
    for language in [AstLanguage::C, AstLanguage::Cpp] {
        let ast = parse(language, source);
        assert_eq!(
            ast.render(AstRenderOptions {
                include_function_bodies: true,
                ..Default::default()
            }),
            source.trim_end()
        );
    }
}

#[test]
fn cpp_namespaces_preserve_qualified_selection() {
    let source = "namespace One {\n    int run() { return 1; }\n}\nnamespace Two {\n    int run() { return 2; }\n}\nnamespace Outer::Inner {\n    class Box {};\n}\n";
    let ast = parse(AstLanguage::Cpp, source);
    assert_eq!(
        select(&ast, "One.run").body.as_deref(),
        Some("int run() { return 1; }")
    );
    assert_eq!(
        select(&ast, "Two.run").body.as_deref(),
        Some("int run() { return 2; }")
    );
    assert_eq!(select(&ast, "Outer.Inner.Box").name.as_deref(), Some("Box"));
}

#[test]
fn csharp_block_and_file_namespaces_preserve_scope_and_complete_bodies() {
    let source = "namespace One {\n    class Box {\n        public void Run() {}\n    }\n}\nnamespace Two {\n    class Box {\n        public void Run() {}\n    }\n}\n";
    let ast = parse(AstLanguage::CSharp, source);
    assert_eq!(
        select(&ast, "One.Box.Run").body.as_deref(),
        Some("public void Run() {}")
    );
    assert_ne!(
        select(&ast, "One.Box").location,
        select(&ast, "Two.Box").location
    );
    let source = "using System;\nnamespace One.Two;\nclass Box {\n    public void Run() {}\n}\n";
    let ast = parse(AstLanguage::CSharp, source);
    assert_eq!(select(&ast, "One.Two.Box.Run").kind, AstItemKind::Function);
    let namespace = select(&ast, "One.Two");
    assert_eq!(namespace.location.display(), "1-5");
    assert_eq!(
        namespace.body.as_deref(),
        Some(source.strip_prefix("using System;\n").unwrap().trim_end())
    );
    assert_eq!(
        ast.render(AstRenderOptions {
            include_type_bodies: true,
            ..Default::default()
        }),
        source.strip_prefix("using System;\n").unwrap().trim_end()
    );
}
