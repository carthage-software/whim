//! Source provenance retained for throwable diagnostics.

use std::rc::Rc;

use whim_span::Span;
use whim_syn::diagnostic;

use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::heap::handle::ManagedRef;
use crate::value::object::InstanceObject;
use crate::value::weak::WeakReference;

use crate::engine::Engine;

#[derive(Clone)]
pub(crate) struct DiagnosticLabel {
    pub(crate) span: Span,
    pub(crate) message: String,
}

/// Source annotations without forcing the common single-label case through a
/// heap allocation. Compiler diagnostics may still carry several locations.
#[derive(Clone)]
pub(crate) enum DiagnosticLabels {
    Single(DiagnosticLabel),
    Multiple(Vec<DiagnosticLabel>),
}

/// A source file and the exact ranges relevant to one exception.
#[derive(Clone)]
pub(crate) struct DiagnosticOrigin {
    pub(crate) path: Atom,
    pub(crate) source: Rc<str>,
    pub(crate) labels: DiagnosticLabels,
}

pub(crate) struct ExceptionDiagnostic {
    /// The exception identity without a strong edge back to it.
    pub(crate) target: ManagedRef<WeakReference>,
    pub(crate) origin: DiagnosticOrigin,
    pub(crate) note: Option<String>,
}

impl ExceptionDiagnostic {
    pub(crate) fn belongs_to(&self, instance: &ManagedRef<InstanceObject>) -> bool {
        self.target
            .upgrade()
            .is_some_and(|target| target.ptr_eq(instance))
    }
}

impl Engine {
    pub(crate) fn diagnostic_origin(
        &self,
        path: &Atom,
        label: DiagnosticLabel,
    ) -> Option<DiagnosticOrigin> {
        let source = self.sources.get(path)?.to_rc();
        Some(DiagnosticOrigin {
            path: path.clone(),
            source,
            labels: DiagnosticLabels::Single(label),
        })
    }

    /// Associates `origin` with a throwable object without retaining that
    /// object strongly.
    pub(crate) fn record_exception_origin(&mut self, value: &Value, origin: DiagnosticOrigin) {
        self.record_exception_origin_with_note(value, origin, None);
    }

    pub(crate) fn record_exception_origin_with_note(
        &mut self,
        value: &Value,
        origin: DiagnosticOrigin,
        note: Option<String>,
    ) {
        let Some(instance) = value.as_object() else {
            return;
        };
        if !self.is_throwable_instance(instance.class()) {
            return;
        }

        self.exception_diagnostics
            .retain(|_, diagnostic| diagnostic.target.upgrade().is_some());
        let address = instance.raw_box().addr().get();
        self.exception_diagnostics.insert(
            address,
            ExceptionDiagnostic {
                target: WeakReference::new(instance),
                origin,
                note,
            },
        );
    }

    pub(crate) fn exception_origin(
        &self,
        instance: &ManagedRef<InstanceObject>,
    ) -> Option<&DiagnosticOrigin> {
        let address = instance.raw_box().addr().get();
        let diagnostic = self.exception_diagnostics.get(&address)?;
        diagnostic
            .belongs_to(instance)
            .then_some(&diagnostic.origin)
    }

    pub(crate) fn exception_note(&self, instance: &ManagedRef<InstanceObject>) -> Option<&str> {
        let address = instance.raw_box().addr().get();
        let diagnostic = self.exception_diagnostics.get(&address)?;
        if !diagnostic.belongs_to(instance) {
            return None;
        }

        diagnostic.note.as_deref()
    }

    pub(crate) fn render_origin(origin: &DiagnosticOrigin, color: bool) -> String {
        let labels: Vec<(Span, &str)> = match &origin.labels {
            DiagnosticLabels::Single(label) => vec![(label.span, label.message.as_str())],
            DiagnosticLabels::Multiple(labels) => labels
                .iter()
                .map(|label| (label.span, label.message.as_str()))
                .collect(),
        };
        let path = origin.path.to_string_lossy();
        diagnostic::render_with_color(&origin.source, &path, &labels, color)
    }
}
