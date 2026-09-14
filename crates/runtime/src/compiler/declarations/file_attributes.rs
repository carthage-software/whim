use whim_syn::cst::node::Node;
use whim_syn::cst::statement::Statement;

use crate::bytecode::unit::CompiledUnit;
use crate::compiler::declarations::Array;
use crate::compiler::declarations::Region;
use crate::compiler::declarations::functions::DeclarationContext;
use crate::compiler::declarations::functions::compile_attribute;
use crate::compiler::emit::Scope;
use crate::compiler::error::CompileError;

pub(super) fn collect(
    context: &Array<'_, '_>,
    statement: &Statement<'_>,
    region: &mut Region<'_, '_>,
    unit: &mut CompiledUnit,
) -> Result<(), CompileError> {
    let scope = Scope {
        heap: context.heap,
        runtime_path: context.runtime_path,
        line_starts: context.line_starts,
        resolver: &region.resolver,
        class: None,
        binders: Vec::new(),
        forbidden_binders: Vec::new(),
        generics: context.generics,
        embedded_files: context.embedded_files,
        trusted_returns: context.trusted_returns,
    };

    let mut nodes = vec![Node::Statement(statement)];
    while let Some(node) = nodes.pop() {
        if let Node::FileAttributeList(list) = node {
            for attribute in &list.attributes {
                region.file_attributes.push(compile_attribute(
                    context.heap,
                    &scope,
                    attribute,
                    context.path,
                    context.source_text,
                    &mut DeclarationContext::for_unit(unit),
                )?);
            }
        }

        node.visit_children(&mut |child| nodes.push(child));
    }

    Ok(())
}
