use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// SPEC §7.3. Рендеринг (ariadne) живёт в aura-cli; ядро только формирует данные.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub primary: (Span, String),
    pub secondary: Vec<(Span, String)>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span, label: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary: (span, label.into()),
            secondary: Vec::new(),
            help: None,
        }
    }
}
