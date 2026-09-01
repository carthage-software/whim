//! Reinterpreting a parsed expression as a restricted assignment target.

use crate::arena::Arena;
use crate::arena::Vec;
use whim_span::HasSpan;

use crate::cst::access::Access;
use crate::cst::array::DictEntry;
use crate::cst::array::DictExpression;
use crate::cst::array::TupleElement;
use crate::cst::array::TupleExpression;
use crate::cst::binding::BindingTarget;
use crate::cst::binding::DictBindingTarget;
use crate::cst::binding::ElementBindingTarget;
use crate::cst::binding::EntryBindingTarget;
use crate::cst::binding::TrailingBindingTarget;
use crate::cst::binding::TupleBindingTarget;
use crate::cst::expression::Expression;
use crate::cst::operation::AssignmentOperator;
use crate::cst::operation::AssignmentTarget;
use crate::cst::operation::DestructureDefault;
use crate::cst::operation::DestructureRest;
use crate::cst::operation::DestructureTarget;
use crate::cst::operation::DictDestructure;
use crate::cst::operation::DictDestructureEntry;
use crate::cst::operation::TupleDestructure;
use crate::cst::sequence::TokenSeparatedSequence;
use crate::error::ParseError;
use crate::parser::Parser;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn expression_to_assignment_target(
        &self,
        expression: &Expression<'arena>,
    ) -> Result<AssignmentTarget<'arena>, ParseError> {
        match expression {
            Expression::Variable(variable) => Ok(AssignmentTarget::Variable(*variable)),
            Expression::Access(Access::Property(access)) => {
                Ok(AssignmentTarget::Property(access.clone()))
            }
            Expression::Access(Access::StaticProperty(access)) => {
                Ok(AssignmentTarget::StaticProperty(access.clone()))
            }
            Expression::ArrayAccess(access) => Ok(AssignmentTarget::ArrayIndex(access.clone())),
            Expression::ArrayAppend(append) => Ok(AssignmentTarget::ArrayAppend(append.clone())),
            Expression::Tuple(tuple) => {
                Ok(AssignmentTarget::Tuple(self.tuple_to_destructure(tuple)?))
            }
            Expression::Dict(dict) => Ok(AssignmentTarget::Dict(self.dict_to_destructure(dict)?)),
            Expression::Parenthesized(parenthesized) => {
                self.expression_to_assignment_target(parenthesized.expression)
            }
            _ => Err(ParseError::InvalidAssignmentTarget(expression.span())),
        }
    }

    pub(crate) fn expression_to_bind_target(
        &self,
        expression: &Expression<'arena>,
    ) -> Result<BindingTarget<'arena>, ParseError> {
        match expression {
            Expression::Variable(variable) => Ok(BindingTarget::Variable(*variable)),
            Expression::Tuple(tuple) => Ok(BindingTarget::Tuple(self.tuple_to_bind_target(tuple)?)),
            Expression::Dict(dict) => Ok(BindingTarget::Dict(self.dict_to_bind_target(dict)?)),
            _ => Err(ParseError::InvalidBindTarget(expression.span())),
        }
    }

    fn dict_to_bind_target(
        &self,
        dict: &DictExpression<'arena>,
    ) -> Result<DictBindingTarget<'arena>, ParseError> {
        let mut entries = Vec::new_in(self.arena);
        for entry in dict.entries {
            let DictEntry::Pair(pair) = entry else {
                return Err(ParseError::InvalidBindTarget(entry.span()));
            };
            entries.push(EntryBindingTarget {
                key: pair.key,
                double_arrow: pair.double_arrow,
                target: self.expression_to_bind_target(pair.value)?,
            });
        }

        Ok(DictBindingTarget {
            dict: dict.dict,
            left_bracket: dict.left_bracket,
            entries: TokenSeparatedSequence::from_slices(entries.leak(), dict.entries.tokens),
            right_bracket: dict.right_bracket,
        })
    }

    fn tuple_to_bind_target(
        &self,
        tuple: &TupleExpression<'arena>,
    ) -> Result<TupleBindingTarget<'arena>, ParseError> {
        let mut targets = Vec::new_in(self.arena);
        for element in tuple.elements {
            match element {
                TupleElement::Value(value) => targets.push(ElementBindingTarget::Target(
                    self.expression_to_bind_target(value)?,
                )),
                TupleElement::Rest(rest) => {
                    let target = match rest.value {
                        Some(value) => Some(self.expression_to_bind_target(value)?),
                        None => None,
                    };
                    targets.push(ElementBindingTarget::Rest(TrailingBindingTarget {
                        ellipsis: rest.ellipsis,
                        target,
                    }));
                }
            }
        }

        Ok(TupleBindingTarget {
            left_parenthesis: tuple.left_parenthesis,
            targets: TokenSeparatedSequence::from_slices(targets.leak(), tuple.elements.tokens),
            right_parenthesis: tuple.right_parenthesis,
        })
    }

    fn tuple_to_destructure(
        &self,
        tuple: &TupleExpression<'arena>,
    ) -> Result<TupleDestructure<'arena>, ParseError> {
        let mut targets = Vec::new_in(self.arena);

        for element in tuple.elements {
            match element {
                TupleElement::Value(Expression::Assignment(assignment))
                    if let AssignmentOperator::Assign(equals) = assignment.operator =>
                {
                    targets.push(DestructureTarget::Default(DestructureDefault {
                        target: assignment.target.clone(),
                        equals,
                        value: assignment.value,
                    }));
                }
                TupleElement::Value(value) => targets.push(DestructureTarget::Target(
                    self.expression_to_assignment_target(value)?,
                )),
                TupleElement::Rest(rest) => {
                    let target = match rest.value {
                        Some(value) => Some(self.expression_to_assignment_target(value)?),
                        None => None,
                    };
                    targets.push(DestructureTarget::Rest(DestructureRest {
                        ellipsis: rest.ellipsis,
                        target,
                    }));
                }
            }
        }

        Ok(TupleDestructure {
            left_parenthesis: tuple.left_parenthesis,
            targets: TokenSeparatedSequence::from_slices(targets.leak(), tuple.elements.tokens),
            right_parenthesis: tuple.right_parenthesis,
        })
    }

    fn dict_to_destructure(
        &self,
        dict: &DictExpression<'arena>,
    ) -> Result<DictDestructure<'arena>, ParseError> {
        let mut entries = Vec::new_in(self.arena);
        for entry in dict.entries {
            let DictEntry::Pair(pair) = entry else {
                return Err(ParseError::InvalidAssignmentTarget(entry.span()));
            };
            entries.push(DictDestructureEntry {
                key: pair.key,
                double_arrow: pair.double_arrow,
                target: self.expression_to_assignment_target(pair.value)?,
            });
        }

        Ok(DictDestructure {
            dict: dict.dict,
            left_bracket: dict.left_bracket,
            entries: TokenSeparatedSequence::from_slices(entries.leak(), dict.entries.tokens),
            right_bracket: dict.right_bracket,
        })
    }
}
