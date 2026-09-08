pub use gpui_base::input::{
    Diagnostic, DiagnosticEntry, DiagnosticRelatedInformation, DiagnosticSet, DiagnosticSeverity,
    DiagnosticSummary, DiagnosticTag, RelatedInformation,
};

mod diagnostic_styles;
pub(crate) use diagnostic_styles::*;

pub(crate) fn input_highlighter_factory() -> gpui_base::input::InputHighlighterFactory {
    std::rc::Rc::new(|_| None)
}

mod wasm_stub;
pub use wasm_stub::*;
