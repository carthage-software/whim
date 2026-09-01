//! Depth-first traversal of the CST via [`Node`].

use crate::arena::Arena;
use crate::arena::Vec as ArenaVec;
use crate::cst::node::Node;

/// Whether a traversal descends into a node's children after visiting it.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Flow {
    Descend,
    Skip,
}

/// A depth-first visitor over the CST.
pub trait Visitor<'ast, 'arena> {
    /// Called on entering `node`, before its children.
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        let _ = node;

        Flow::Descend
    }

    /// Called on leaving `node`, after its children have been visited.
    /// The default does nothing.
    fn leave(&mut self, node: Node<'ast, 'arena>) {
        let _ = node;
    }
}

enum Step<'ast, 'arena> {
    Enter(Node<'ast, 'arena>),
    Leave(Node<'ast, 'arena>),
}

/// Walks the tree rooted at `node`, depth-first, driving `visitor`.
pub fn walk<'ast, 'arena, V>(node: Node<'ast, 'arena>, visitor: &mut V)
where
    V: Visitor<'ast, 'arena> + ?Sized,
{
    let mut steps = vec![Step::Enter(node)];
    let mut children = Vec::new();

    while let Some(step) = steps.pop() {
        match step {
            Step::Enter(node) => {
                if visitor.enter(node) == Flow::Skip {
                    visitor.leave(node);

                    continue;
                }

                steps.push(Step::Leave(node));
                children.clear();
                node.visit_children(&mut |child| children.push(child));
                steps.extend(children.iter().rev().map(|child| Step::Enter(*child)));
            }
            Step::Leave(node) => visitor.leave(node),
        }
    }
}

/// How deep a tree is, and where its deepest path ends.
#[derive(Debug, Clone, Copy)]
pub struct DeepestPath<'ast, 'arena> {
    /// The number of nodes on the longest path from the root to a leaf,
    /// counting both ends: a lone node is one level.
    pub levels: usize,
    /// The last node on that path. It has no children.
    pub end: Node<'ast, 'arena>,
}

/// Measures the tree rooted at `node`.
#[must_use]
pub fn deepest_path<'ast, 'arena>(node: Node<'ast, 'arena>) -> DeepestPath<'ast, 'arena> {
    let mut stack = vec![(node, 1usize)];
    let mut deepest = DeepestPath {
        levels: 1,
        end: node,
    };

    while let Some((node, levels)) = stack.pop() {
        if levels > deepest.levels {
            deepest = DeepestPath { levels, end: node };
        }

        node.visit_children(&mut |child| stack.push((child, levels + 1)));
    }

    deepest
}

/// Measures a tree using arena-backed traversal storage.
#[must_use]
pub fn deepest_path_in<'ast, 'arena, A>(
    arena: &'arena A,
    node: Node<'ast, 'arena>,
) -> DeepestPath<'ast, 'arena>
where
    A: Arena,
{
    let mut stack = ArenaVec::new_in(arena);
    stack.push((node, 1usize));
    let mut deepest = DeepestPath {
        levels: 1,
        end: node,
    };

    while let Some((node, levels)) = stack.pop() {
        if levels > deepest.levels {
            deepest = DeepestPath { levels, end: node };
        }

        node.visit_children(&mut |child| stack.push((child, levels + 1)));
    }

    deepest
}

#[cfg(test)]
mod tests {
    use crate::arena::LocalArena;
    use crate::cst::node::Node;
    use crate::cst::walker::Visitor;
    use crate::cst::walker::deepest_path;
    use crate::cst::walker::walk;
    use crate::parser::parse;
    use crate::unreachable_invariant;

    struct NoOpVisitor;

    impl Visitor<'_, '_> for NoOpVisitor {}

    #[test]
    fn walk_and_deepest_path_handle_a_long_left_nested_chain() {
        let arena = LocalArena::new();
        let source = format!("$a{};", "+1".repeat(1_000));
        let program = match parse(&arena, &source) {
            Ok(program) => program,
            // SAFETY: the fixture source parses.
            Err(_) => unsafe { unreachable_invariant("fixture source parses") },
        };

        walk(Node::Program(program), &mut NoOpVisitor);

        let deepest = deepest_path(Node::Program(program));
        assert!(deepest.levels > 1_000);
        assert!(
            deepest.end.children().is_empty(),
            "the deepest node is a leaf"
        );
    }
}
