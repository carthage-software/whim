//! The frames a statement records its jumps on.

use whim_syn::cst::node::Node;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::compiler::emit::Block;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::CompileErrorKind;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::Statement;
use crate::unreachable_invariant;

/// The control-flow frames a statement compiles under, innermost last.
#[derive(Default)]
pub(in crate::compiler::emit) struct ControlFlow<'arena> {
    pub(in crate::compiler::emit) frames: Vec<ControlFrame<'arena>>,
}

/// One enclosing control-flow construct.
pub(in crate::compiler::emit) enum ControlFrame<'arena> {
    Loop(LoopFrame),
    /// An enclosing `finally` block and its uncovered ranges.
    Finally {
        cleanup: Cleanup<'arena>,
        holes: Vec<(u32, u32)>,
    },
}

/// A cleanup body duplicated when control leaves a protected region.
#[derive(Clone)]
pub(in crate::compiler::emit) enum Cleanup<'arena> {
    Finally(Block<'arena>),
    Using(usize),
}

/// Splits a protected range around the holes punched into it.
pub(in crate::compiler::emit) fn subtract_holes(
    start: u32,
    end: u32,
    holes: &[(u32, u32)],
    output: &mut Vec<(u32, u32)>,
) {
    let mut cursor = start;
    for &(hole_start, hole_end) in holes {
        if hole_start > cursor {
            output.push((cursor, hole_start));
        }
        cursor = cursor.max(hole_end);
    }
    if cursor < end {
        output.push((cursor, end));
    }
}

/// The pending jumps of one loop, patched when the loop closes.
pub(in crate::compiler::emit) struct LoopFrame {
    /// The `break` jumps, patched to just after the loop.
    pub(in crate::compiler::emit) break_jumps: Vec<u32>,
    /// The `continue` jumps, patched to the loop's continuation point.
    pub(in crate::compiler::emit) continue_jumps: Vec<u32>,
    /// The assignment state at each abrupt exit.
    pub(in crate::compiler::emit) escape_states: Vec<Vec<bool>>,
}

impl LoopFrame {
    pub(in crate::compiler::emit) const fn new() -> Self {
        Self {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            escape_states: Vec::new(),
        }
    }
}

/// Which loop jump a statement performs.
#[derive(Clone, Copy)]
pub(in crate::compiler::emit) enum LoopJump {
    Break,
    Continue,
}

/// Pops the finally frame a `try` pushed, returning its punched holes.
pub(in crate::compiler::emit) fn pop_finally_holes(
    flow: &mut ControlFlow<'_>,
    pushed: bool,
) -> Vec<(u32, u32)> {
    if !pushed {
        return Vec::new();
    }
    match flow.frames.pop() {
        Some(ControlFrame::Finally { holes, .. }) => holes,
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a try closes the frame it opened") },
    }
}

/// Pops the loop frame a loop construct pushed.
pub(in crate::compiler::emit) fn pop_loop_frame(flow: &mut ControlFlow<'_>) -> LoopFrame {
    match flow.frames.pop() {
        Some(ControlFrame::Loop(frame)) => frame,
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("a loop closes the frame it opened") },
    }
}

/// Applies the `finally` gates to every statement nested inside `statements`.
pub(in crate::compiler::emit) fn scan_statements(
    statements: &[Statement<'_>],
) -> Result<(), CompileError> {
    let mut visitor = FinallyGates {
        outcome: Ok(()),
        loop_depth: None,
        cleanup_name: "finally",
    };
    for statement in statements {
        walk(Node::Statement(statement), &mut visitor);
    }

    visitor.outcome
}

/// Walks a `finally` block: `return` cannot appear, and `break` or
/// `continue` cannot escape past the loops nested inside the block.
fn scan_finally_block(statements: &[Statement<'_>], loop_depth: u64) -> Result<(), CompileError> {
    scan_cleanup_block(statements, loop_depth, "finally")
}

fn scan_cleanup_block(
    statements: &[Statement<'_>],
    loop_depth: u64,
    cleanup_name: &'static str,
) -> Result<(), CompileError> {
    let mut visitor = FinallyGates {
        outcome: Ok(()),
        loop_depth: Some(loop_depth),
        cleanup_name,
    };
    for statement in statements {
        walk(Node::Statement(statement), &mut visitor);
    }

    visitor.outcome
}

struct FinallyGates {
    outcome: Result<(), CompileError>,
    loop_depth: Option<u64>,
    cleanup_name: &'static str,
}

impl FinallyGates {
    /// Records `error` unless a violation was already found.
    fn fail(&mut self, error: CompileError) -> Flow {
        if self.outcome.is_ok() {
            self.outcome = Err(error);
        }

        Flow::Skip
    }

    fn escapes(&self, level: u64) -> bool {
        self.loop_depth.is_some_and(|depth| level > depth)
    }
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for FinallyGates {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        if self.outcome.is_err() {
            return Flow::Skip;
        }

        match node {
            Node::Closure(_)
            | Node::ShortClosure(_)
            | Node::Function(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Enum(_)
            | Node::TryFinallyClause(_) => Flow::Skip,
            Node::Try(try_statement) => {
                if let Some(finally_clause) = &try_statement.finally_clause
                    && let Err(error) = scan_finally_block(finally_clause.block.statements, 0)
                {
                    return self.fail(error);
                }

                Flow::Descend
            }
            Node::Return(return_statement) if self.loop_depth.is_some() => {
                self.fail(CompileError::new(
                    CompileErrorKind::ReturnInsideFinally,
                    format!(
                        "`return` cannot appear inside a `{}` block",
                        self.cleanup_name
                    ),
                    return_statement.span(),
                ))
            }
            Node::Break(break_statement) => {
                let level = break_statement
                    .level
                    .as_ref()
                    .map_or(1, |literal| literal.value);
                if self.escapes(level) {
                    return self.fail(CompileError::new(
                        CompileErrorKind::LoopJumpEscapesFinally,
                        format!(
                            "`break` cannot transfer control out of a `{}` block",
                            self.cleanup_name
                        ),
                        break_statement.span(),
                    ));
                }

                Flow::Descend
            }
            Node::Continue(continue_statement) => {
                let level = continue_statement
                    .level
                    .as_ref()
                    .map_or(1, |literal| literal.value);
                if self.escapes(level) {
                    return self.fail(CompileError::new(
                        CompileErrorKind::LoopJumpEscapesFinally,
                        format!(
                            "`continue` cannot transfer control out of a `{}` block",
                            self.cleanup_name
                        ),
                        continue_statement.span(),
                    ));
                }

                Flow::Descend
            }
            Node::While(_) | Node::DoWhile(_) | Node::For(_) | Node::Foreach(_) => {
                let Some(depth) = self.loop_depth else {
                    return Flow::Descend;
                };
                let body = match node {
                    Node::While(statement) => &statement.body,
                    Node::DoWhile(statement) => &statement.body,
                    Node::For(statement) => &statement.body,
                    Node::Foreach(statement) => &statement.body,
                    // SAFETY: the surrounding invariant makes this path unreachable.
                    _ => unsafe {
                        unreachable_invariant("the arm matched one of the four loop statements")
                    },
                };
                if let Err(error) =
                    scan_cleanup_block(body.statements, depth + 1, self.cleanup_name)
                {
                    return self.fail(error);
                }

                Flow::Skip
            }
            _ => Flow::Descend,
        }
    }
}
