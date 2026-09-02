//! The in-unit type-alias dependency graph and its cycle check.

use hashbrown::HashMap;
use whim_span::Span;

use whim_syn::cst::atom::Identifier;
use whim_syn::cst::node::Node;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::compiler::names::Resolver;
use crate::compiler::types::AliasEdge;
use crate::compiler::types::AliasGraph;
use crate::compiler::types::GenericTable;

struct AliasReferences<'context, 'arena> {
    resolver: &'context Resolver,
    generics: &'context GenericTable<'arena>,
    binders: &'context [String],
    out: &'context mut Vec<AliasEdge>,
    union_depth: usize,
    array_depth: usize,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for AliasReferences<'_, '_> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        match node {
            Node::UnionType(_) => self.union_depth += 1,
            Node::VecType(_) | Node::DictType(_) | Node::TupleType(_) => {
                self.array_depth += 1;
            }
            _ => {}
        }

        if let Node::NamedType(named) = node
            && !is_binder_name(&named.identifier, self.binders)
        {
            let resolved = self.resolver.resolve_text(&named.identifier);
            if self
                .generics
                .get(&resolved)
                .is_some_and(|declaration| declaration.is_alias)
            {
                let edge = AliasEdge {
                    target: resolved,
                    through_union: self.union_depth > 0,
                    through_array: self.array_depth > 0,
                };
                if !self.out.contains(&edge) {
                    self.out.push(edge);
                }
            }
        }

        Flow::Descend
    }

    fn leave(&mut self, node: Node<'ast, 'arena>) {
        match node {
            Node::UnionType(_) => self.union_depth -= 1,
            Node::VecType(_) | Node::DictType(_) | Node::TupleType(_) => {
                self.array_depth -= 1;
            }
            _ => {}
        }
    }
}

pub(in crate::compiler) fn collect_alias_references(
    resolver: &Resolver,
    generics: &GenericTable<'_>,
    source: &Type<'_>,
    binders: &[String],
    out: &mut Vec<AliasEdge>,
) {
    walk(
        Node::Type(source),
        &mut AliasReferences {
            resolver,
            generics,
            binders,
            out,
            union_depth: 0,
            array_depth: 0,
        },
    );
}

fn is_binder_name(identifier: &Identifier<'_>, binders: &[String]) -> bool {
    matches!(
        identifier,
        Identifier::Local(local) if binders.iter().any(|binder| binder == local.value)
    )
}

pub(in crate::compiler) struct AliasCycle {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Visit {
    Active,
    Complete,
}

#[derive(Clone, Copy)]
enum RequiredEdge {
    WithoutUnion,
    WithoutArray,
}

struct Frame<'graph> {
    name: &'graph str,
    next_edge: usize,
}

pub(in crate::compiler) fn find_alias_cycle(aliases: &AliasGraph) -> Option<AliasCycle> {
    let path = find_filtered_cycle(aliases, RequiredEdge::WithoutUnion)
        .or_else(|| find_filtered_cycle(aliases, RequiredEdge::WithoutArray))?;
    let mut names = path[..path.len() - 1].to_vec();
    let latest = names
        .iter()
        .enumerate()
        .max_by_key(|(_, name)| aliases.get(name).map_or(0, |node| node.order))
        .map_or(0, |(index, _)| index);
    names.rotate_left(latest);
    let span = aliases.get(names[0])?.span;
    names.push(names[0]);

    Some(AliasCycle {
        path: names.into_iter().map(str::to_string).collect(),
        span,
    })
}

fn find_filtered_cycle(aliases: &AliasGraph, required: RequiredEdge) -> Option<Vec<&str>> {
    let mut visits = HashMap::new();
    let mut positions = HashMap::new();
    let mut path = Vec::new();
    let mut stack = Vec::new();

    for root in aliases.names() {
        if visits.contains_key(root) {
            continue;
        }

        visits.insert(root, Visit::Active);
        positions.insert(root, 0);
        path.push(root);
        stack.push(Frame {
            name: root,
            next_edge: 0,
        });

        while let Some(frame) = stack.last_mut() {
            let node = aliases.get(frame.name)?;
            let next = node.edges[frame.next_edge..]
                .iter()
                .enumerate()
                .find(|(_, edge)| edge_is_allowed(edge, required));
            let Some((offset, edge)) = next else {
                let completed = stack.pop()?.name;
                visits.insert(completed, Visit::Complete);
                positions.remove(completed);
                path.pop();
                continue;
            };

            frame.next_edge += offset + 1;
            let target = edge.target.as_str();
            if aliases.get(target).is_none() {
                continue;
            }

            match visits.get(target).copied() {
                Some(Visit::Complete) => {}
                Some(Visit::Active) => {
                    let start = positions.get(target).copied()?;
                    let mut cycle = path[start..].to_vec();
                    cycle.push(target);
                    return Some(cycle);
                }
                None => {
                    visits.insert(target, Visit::Active);
                    positions.insert(target, path.len());
                    path.push(target);
                    stack.push(Frame {
                        name: target,
                        next_edge: 0,
                    });
                }
            }
        }
    }

    None
}

const fn edge_is_allowed(edge: &AliasEdge, required: RequiredEdge) -> bool {
    match required {
        RequiredEdge::WithoutUnion => !edge.through_union,
        RequiredEdge::WithoutArray => !edge.through_array,
    }
}
