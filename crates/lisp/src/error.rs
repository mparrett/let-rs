//! Structured errors carrying an optional source position.
//!
//! Every error in the engine used to be a bare `String`, so a missing
//! paren in a 40-line prelude reported exactly `unclosed (` and an
//! unbound variable reported exactly `unbound variable: foo`. Both are
//! true and neither is actionable. See ADR-039 (which implements
//! ADR-022's design, plus the slice of its deferred Phase 2 that covers
//! variable references and call sites).
//!
//! Two rules keep the plumbing honest:
//!
//! - **Innermost span wins.** [`LispErr::with_span`] only fills a span
//!   that is still `None`, so an error raised deep inside a form keeps
//!   its own position as it propagates outward through enclosing forms.
//! - **`None` is a real answer.** Macro-synthesized code and
//!   host-constructed forms have no source text. They report unpositioned
//!   errors that render exactly as they did before spans existed.

use std::fmt;

/// A source position: 1-indexed line and column, plus a length in
/// characters for highlight rendering.
///
/// `line`/`col` rather than a byte offset because every host wants to
/// display them (ADR-022, alternative 3); converting once at error
/// construction is cheaper than making each host do it. `col` counts
/// characters, not bytes, so a caret lines up under non-ASCII source —
/// which this project needs, since rune and trigram literals are
/// multi-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// 1-indexed line.
    pub line: u32,
    /// 1-indexed column, counted in characters.
    pub col: u32,
    /// Length in characters, for underlining. A compound form spans just
    /// its opening delimiter — pointing at the `(` of the offending form
    /// is what a reader wants, and a line:col pair can't describe a
    /// region that crosses a newline anyway.
    pub len: u32,
}

impl Span {
    pub fn new(line: u32, col: u32, len: u32) -> Span {
        Span { line, col, len }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// An engine error: a message, and where in the source it came from if
/// that's known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LispErr {
    pub msg: String,
    pub span: Option<Span>,
}

impl LispErr {
    /// An error with no source position. Same shape as the bare strings
    /// this type replaced.
    pub fn new(msg: impl Into<String>) -> LispErr {
        LispErr {
            msg: msg.into(),
            span: None,
        }
    }

    /// An error at a known position.
    pub fn at(msg: impl Into<String>, span: Span) -> LispErr {
        LispErr {
            msg: msg.into(),
            span: Some(span),
        }
    }

    /// An error at a position that may not be known — the common shape
    /// when the position comes from a `Datum`, which has `None` for
    /// macro-synthesized forms.
    pub fn maybe_at(msg: impl Into<String>, span: Option<Span>) -> LispErr {
        LispErr {
            msg: msg.into(),
            span,
        }
    }

    /// Attach `span` **only if this error doesn't already have one.**
    ///
    /// This is the combinator that makes propagation work. Compiling
    /// `(lambda (x) (if 1 2))` fails inside the `if`, and the `lambda`
    /// site sees that error on its way out; overwriting the span there
    /// would walk the position outward to the top-level form, which is
    /// how you end up reporting every error at line 1. Callers can wrap
    /// freely because the innermost annotation is the one that sticks.
    pub fn with_span(mut self, span: Option<Span>) -> LispErr {
        if self.span.is_none() {
            self.span = span;
        }
        self
    }

    /// Render the error with the offending source line and a caret run
    /// under the span:
    ///
    /// ```text
    /// 3:8: unbound variable: mama
    ///   |
    /// 3 |   (+ 1 mama)
    ///   |        ^^^^
    /// ```
    ///
    /// Falls back to plain [`Display`](fmt::Display) when there's no
    /// span, or when `src` doesn't have the line the span names (a host
    /// that reports an error against different text than it evaluated).
    ///
    /// Takes `src` rather than storing it: errors outlive the string they
    /// came from, and an engine that retained source per error would keep
    /// every REPL line alive for the life of the `Vm`.
    pub fn render(&self, src: &str) -> String {
        render_span(src, self.span, &self.msg)
    }
}

/// Point at a place in `src` with a message above it and a caret run
/// under it:
///
/// ```text
/// 3:8: unbound variable: mama
///   |
/// 3 |   (+ 1 mama)
///   |        ^^^^
/// ```
///
/// Shared by [`LispErr::render`] and by hosts pointing at something that
/// *isn't* an error — a stepping debugger showing where a paused machine
/// sits uses the same view. Falls back to `msg` alone when there's no
/// span, or when `src` doesn't have the line the span names (a host
/// rendering against different text than it evaluated).
pub fn render_span(src: &str, span: Option<Span>, msg: &str) -> String {
    let Some(span) = span else {
        return msg.to_string();
    };
    let Some(line_text) = src.lines().nth(span.line as usize - 1) else {
        return format!("{span}: {msg}");
    };

    // Gutter width from the line number, so the bars line up.
    let num = span.line.to_string();
    let pad = " ".repeat(num.len());

    // `col` and `len` are in characters; the caret run has to be measured
    // the same way to sit under multi-byte source. Tabs are copied
    // through rather than counted as one column, so the caret tracks
    // however wide the reader's terminal renders them.
    let lead: String = line_text
        .chars()
        .take(span.col as usize - 1)
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let carets = "^".repeat(span.len.max(1) as usize);

    format!("{span}: {msg}\n{pad} |\n{num} | {line_text}\n{pad} | {lead}{carets}")
}

impl fmt::Display for LispErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.span {
            Some(s) => write!(f, "{s}: {}", self.msg),
            None => write!(f, "{}", self.msg),
        }
    }
}

impl std::error::Error for LispErr {}

// Every internal site that still returns a bare `String` — the prims,
// `Env::set`, host callbacks — propagates through `?` unchanged and lands
// here with no span. Prim errors then get the *call site's* span attached
// by `step::apply`, which is the position a reader actually wants: the
// prim itself has no source text.
impl From<String> for LispErr {
    fn from(msg: String) -> LispErr {
        LispErr::new(msg)
    }
}

impl From<&str> for LispErr {
    fn from(msg: &str) -> LispErr {
        LispErr::new(msg)
    }
}

impl From<LispErr> for String {
    fn from(e: LispErr) -> String {
        e.to_string()
    }
}
