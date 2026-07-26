//! Phase 2 acceptance criterion (SPEC §8): the reference manifest parses into the expected AST shape.

use aura_lang::lexer::Lexer;
use aura_lang::parser::ast::*;
use aura_lang::parser::Parser;

const MANIFEST: &str = include_str!("fixtures/production_deploy.aura");

fn parse() -> Module<'static> {
    let toks = Lexer::new(MANIFEST, 0).tokenize().expect("lex ok");
    Parser::new(toks)
        .parse_module()
        .unwrap_or_else(|d| panic!("parse failed: {d:#?}"))
}

#[test]
fn manifest_structure() {
    let m = parse();
    assert_eq!(m.imports.len(), 2);
    assert_eq!(
        m.imports[0].source,
        ImportSource::Registry {
            path: "github/actions/rust-cache",
            version: "v1.2"
        }
    );
    assert_eq!(
        m.imports[1].source,
        ImportSource::File("templates/k8s_defaults.aura")
    );

    // Top level: 5 assigns, type, def, domain
    assert!(m
        .stmts
        .iter()
        .any(|s| matches!(s, Stmt::TypeDecl(t) if t.name == "ServiceMeta" && t.fields.len() == 2)));
    assert!(m.stmts.iter().any(
        |s| matches!(s, Stmt::FuncDecl { name: "build_labels", params, .. } if params.len() == 2)
    ));

    let Some(Stmt::Block(domain)) = m.stmts.iter().find(|s| matches!(s, Stmt::Block(_))) else {
        panic!("no domain block")
    };
    assert_eq!(domain.kind, BlockKind::Domain);
    assert!(matches!(
        domain.label,
        Expr::Literal(LitValue::Str("production-eu"), _)
    ));

    // shadow shadowing (D7)
    assert!(domain.body.iter().any(|s| matches!(
        s,
        Stmt::Assign {
            name: "global_file_path",
            shadow: true,
            ..
        }
    )));

    // meta: new ServiceMeta ... end (D4, D10 - a property)
    assert!(domain.body.iter().any(|s| matches!(
        s,
        Stmt::Property {
            key: "meta",
            value: Expr::SchemaInstance {
                schema: "ServiceMeta",
                ..
            },
            ..
        }
    )));

    // apps: active_services.map (name, index) -> name: name ... end (D17)
    let apps = domain.body.iter().find_map(|s| match s {
        Stmt::Property {
            key: "apps", value, ..
        } => Some(value),
        _ => None,
    });
    let Some(Expr::MethodCall {
        method: "map",
        lambda: Some(l),
        ..
    }) = apps
    else {
        panic!("apps must be .map with trailing lambda")
    };
    let Expr::Lambda {
        params,
        body: LambdaBody::Block(stmts),
        ..
    } = l.as_ref()
    else {
        panic!("lambda body must be a statement body (D17)")
    };
    assert_eq!(params, &["name", "index"]);
    // The item's own `name` is now an ordinary property, not injected by a keyword.
    assert!(stmts
        .iter()
        .any(|s| matches!(s, Stmt::Property { key: "name", .. })));

    // assert ... , "..." (D5)
    assert!(domain.body.iter().any(|s| matches!(
        s,
        Stmt::Assert {
            message: Some(_),
            ..
        }
    )));
}
