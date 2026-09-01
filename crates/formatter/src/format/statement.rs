//! Formatting for the program root, statements, and control flow.

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::Program;
use whim_syn::cst::control_flow::DoWhile;
use whim_syn::cst::control_flow::Else;
use whim_syn::cst::control_flow::ElseBody;
use whim_syn::cst::control_flow::For;
use whim_syn::cst::control_flow::Foreach;
use whim_syn::cst::control_flow::ForeachTarget;
use whim_syn::cst::control_flow::If;
use whim_syn::cst::control_flow::Try;
use whim_syn::cst::control_flow::TryCatchClause;
use whim_syn::cst::control_flow::TryElseClause;
use whim_syn::cst::control_flow::TryFinallyClause;
use whim_syn::cst::control_flow::While;
use whim_syn::cst::statement::Block;
use whim_syn::cst::statement::ExpressionStatement;
use whim_syn::cst::statement::FinalLocal;
use whim_syn::cst::statement::Statement;
use whim_syn::cst::statement::Using;
use whim_syn::cst::statement::UsingBinding;

use crate::document::Document;
use crate::document::Group;
use crate::format::Format;
use crate::format::FormatterState;

impl<'arena, A> Format<'arena, A> for Program<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let mut parts = f.vec();

        if let Some(shebang) = f.shebang {
            parts.push(f.text(shebang));
            parts.push(f.hard_line());
            if let Some(first) = self.statements.first()
                && f.count_newlines(shebang.len() as u32, first.span().start.offset) >= 2
            {
                parts.push(f.hard_line());
            }
        }

        let items = f.format_sequence(self.statements, self.source_text.len() as u32);
        let has_content = !parts.is_empty() || !items.is_empty();
        parts.extend(items);

        if has_content {
            parts.push(f.hard_line());
        }

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Statement<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Statement::Namespace(node) => node.format(f),
            Statement::Use(node) => node.format(f),
            Statement::Class(node) => node.format(f),
            Statement::Interface(node) => node.format(f),
            Statement::Enum(node) => node.format(f),
            Statement::Function(node) => node.format(f),
            Statement::Constant(node) => node.format(f),
            Statement::TypeAlias(node) => node.format(f),
            Statement::Newtype(node) => node.format(f),
            Statement::Block(node) => node.format(f),
            Statement::If(node) => node.format(f),
            Statement::While(node) => node.format(f),
            Statement::DoWhile(node) => node.format(f),
            Statement::For(node) => node.format(f),
            Statement::Foreach(node) => node.format(f),
            Statement::Try(node) => node.format(f),
            Statement::Using(node) => node.format(f),
            Statement::FinalLocal(node) => node.format(f),
            Statement::Expression(node) => node.format(f),
            Statement::Noop(_) => f.empty(),
        }
    }

    #[inline]
    fn should_skip(&self) -> bool {
        self.is_noop()
    }

    #[inline]
    fn blank_line_after_if_multiline(&self) -> bool {
        true
    }
}

impl<'arena, A> Format<'arena, A> for Block<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.format_braced_sequence(self.statements, self.right_brace.start.offset)
    }
}

impl<'arena, A> Format<'arena, A> for ExpressionStatement<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let expression = self.expression.format(f);
        f.concat([expression, f.text(";")])
    }
}

impl<'arena, A> Format<'arena, A> for FinalLocal<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let variable = self.variable.format(f);
        let value = self.value.format(f);

        f.concat([
            f.text("final"),
            f.space(),
            variable,
            f.text(" = "),
            value,
            f.text(";"),
        ])
    }
}

impl<'arena, A> Format<'arena, A> for Using<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let bindings = f.delimited(
            "(",
            self.bindings.as_slice(),
            ")",
            self.right_parenthesis.start.offset,
            false,
        );
        let mut parts = f.vec();
        parts.push(f.text("using"));
        parts.push(f.space());
        parts.push(bindings);
        parts.push(f.space());
        parts.push(self.body.format(f));
        Document::Group(Group::new(parts))
    }
}

impl<'arena, A> Format<'arena, A> for UsingBinding<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let target = self.target.format(f);
        let value = self.value.format(f);
        f.concat([target, f.text(" = "), value])
    }
}

impl<'arena, A> Format<'arena, A> for If<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let condition = self.condition.unparenthesized().format(f);
        let body = self.body.format(f);

        let mut condition_inner = f.vec();
        condition_inner.push(f.soft_line());
        condition_inner.push(condition);

        let mut header = f.vec();
        header.push(f.text("if"));
        header.push(f.space());
        header.push(f.text("("));
        header.push(f.indent_if_break(condition_inner));
        header.push(f.soft_line());
        header.push(f.text(")"));

        let mut parts = f.vec();
        parts.push(Document::Group(Group::new(header)));
        parts.push(f.space());
        parts.push(body);

        if let Some(r#else) = &self.r#else {
            let else_document = r#else.format(f);
            parts.push(else_document);
        }

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Else<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let body = match &self.body {
            ElseBody::Block(block) => block.format(f),
            ElseBody::If(r#if) => r#if.format(f),
        };

        f.concat([f.space(), f.text("else"), f.space(), body])
    }
}

impl<'arena, A> Format<'arena, A> for While<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let condition = self.condition.unparenthesized().format(f);
        let body = self.body.format(f);

        let mut condition_inner = f.vec();
        condition_inner.push(f.soft_line());
        condition_inner.push(condition);

        let mut header = f.vec();
        header.push(f.text("while"));
        header.push(f.space());
        header.push(f.text("("));
        header.push(f.indent_if_break(condition_inner));
        header.push(f.soft_line());
        header.push(f.text(")"));

        f.concat([Document::Group(Group::new(header)), f.space(), body])
    }
}

impl<'arena, A> Format<'arena, A> for DoWhile<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let body = self.body.format(f);
        let condition = self.condition.unparenthesized().format(f);

        f.concat([
            f.text("do"),
            f.space(),
            body,
            f.space(),
            f.text("while"),
            f.space(),
            f.text("("),
            condition,
            f.text(")"),
            f.text(";"),
        ])
    }
}

impl<'arena, A> Format<'arena, A> for For<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let initializations = f.inline_token_sequence(&self.initializations);
        let conditions = f.inline_token_sequence(&self.conditions);
        let body = self.body.format(f);

        let mut parts = f.vec();
        parts.push(f.text("for"));
        parts.push(f.space());
        parts.push(f.text("("));
        parts.push(initializations);
        parts.push(f.text("; "));
        parts.push(conditions);
        parts.push(f.text(";"));
        if !self.increments.as_slice().is_empty() {
            let increments = f.inline_token_sequence(&self.increments);
            parts.push(f.space());
            parts.push(increments);
        }
        parts.push(f.text(")"));
        parts.push(f.space());
        parts.push(body);

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Foreach<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let expression = self.expression.unparenthesized().format(f);
        let target = match &self.target {
            ForeachTarget::Value(value) => value.value.format(f),
            ForeachTarget::KeyValue(key_value) => {
                let key = key_value.key.format(f);
                let value = key_value.value.format(f);
                f.concat([key, f.text(" => "), value])
            }
        };
        let body = self.body.format(f);

        f.concat([
            f.text("foreach"),
            f.space(),
            f.text("("),
            expression,
            f.text(" as "),
            target,
            f.text(")"),
            f.space(),
            body,
        ])
    }
}

impl<'arena, A> Format<'arena, A> for Try<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let block = f.format_expanded_braced_sequence(
            self.block.statements,
            self.block.right_brace.start.offset,
        );

        let mut parts = f.vec();
        parts.push(f.text("try"));
        parts.push(f.space());
        parts.push(block);

        for catch in self.catch_clauses {
            let document = catch.format(f);
            parts.push(document);
        }

        if let Some(r#else) = &self.else_clause {
            let document = r#else.format(f);
            parts.push(document);
        }

        if let Some(finally) = &self.finally_clause {
            let document = finally.format(f);
            parts.push(document);
        }

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for TryCatchClause<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let r#type = self.r#type.unparenthesized().format(f);
        let block = f.format_expanded_braced_sequence(
            self.block.statements,
            self.block.right_brace.start.offset,
        );

        let mut catch_inner = f.vec();
        catch_inner.push(f.soft_line());
        catch_inner.push(r#type);
        if let Some(variable) = &self.variable {
            catch_inner.push(f.space());
            catch_inner.push(f.text(variable.name));
        }

        let mut catch_header = f.vec();
        catch_header.push(f.text("catch"));
        catch_header.push(f.space());
        catch_header.push(f.text("("));
        catch_header.push(f.indent_if_break(catch_inner));
        catch_header.push(f.soft_line());
        catch_header.push(f.text(")"));

        if let Some(guard) = &self.guard {
            let condition = guard.condition.unparenthesized().format(f);
            catch_header.push(f.space());
            catch_header.push(f.text("if"));
            catch_header.push(f.space());
            catch_header.push(f.text("("));
            catch_header.push(condition);
            catch_header.push(f.text(")"));
        }

        let mut parts = f.vec();
        parts.push(f.space());
        parts.push(Document::Group(Group::new(catch_header)));
        parts.push(f.space());
        parts.push(block);

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for TryElseClause<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let block = f.format_expanded_braced_sequence(
            self.block.statements,
            self.block.right_brace.start.offset,
        );
        f.concat([f.space(), f.text("else"), f.space(), block])
    }
}

impl<'arena, A> Format<'arena, A> for TryFinallyClause<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let block = f.format_expanded_braced_sequence(
            self.block.statements,
            self.block.right_brace.start.offset,
        );
        f.concat([f.space(), f.text("finally"), f.space(), block])
    }
}
