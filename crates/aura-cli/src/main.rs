fn main() {
    // Фаза 6 (SPEC §7.2). Пока доступна только лексическая проверка: aura check <file>
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("check"), Some(path)) => {
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: cannot read {path}: {e}");
                    std::process::exit(2);
                }
            };
            match aura_core::lexer::Lexer::new(&src, 0).tokenize() {
                Ok(tokens) => println!("ok: {} tokens", tokens.len()),
                Err(d) => {
                    eprintln!("{}: {} at bytes {}..{}", d.code, d.message, d.primary.0.start, d.primary.0.end);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: aura check <file.aura>");
            std::process::exit(2);
        }
    }
}
