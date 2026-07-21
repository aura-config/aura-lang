pub type SourceId = u32;

/// Byte offsets into the source (SPEC §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub source: SourceId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(source: SourceId, start: usize, end: usize) -> Self {
        Span {
            source,
            start: start as u32,
            end: end as u32,
        }
    }
}
