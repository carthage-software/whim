//! Exact property reads proven by whole-unit type flow.

use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::PropertyReadMode;
use crate::bytecode::instruction::operands::PropertySlot;
use crate::bytecode::unit::Visibility;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::OptimizationStatistics;
use crate::optimizer::analysis::Analysis;
use crate::optimizer::candidates::CandidateSet;
use crate::optimizer::rewrite::plan::RewritePlan;

pub(in crate::optimizer) fn optimize_unit(
    plan: &mut RewritePlan,
    analysis: &Analysis<'_>,
    configuration: OptimizationConfiguration,
    statistics: &mut OptimizationStatistics,
) {
    if !configuration.specialize_property_get {
        return;
    }

    for analyzed in analysis.chunks() {
        if !analyzed.candidates.contains(CandidateSet::PROPERTY) {
            continue;
        }

        for (index, instruction) in analyzed.chunk.code.iter().copied().enumerate() {
            if !plan.is_available(analyzed, index) {
                continue;
            }

            let (destination, object, cache, optional) = match instruction {
                Instruction::PropertyGet {
                    destination,
                    object,
                    cache,
                } => (destination, object, cache, false),
                Instruction::PropertyGetOrNull {
                    destination,
                    object,
                    cache,
                } => (destination, object, cache, true),
                _ => continue,
            };

            let Some(resolved) = analyzed.flow.resolved_property(index, object, cache) else {
                continue;
            };

            if resolved.property.visibility != Visibility::Public
                && analyzed.class_name != Some(&resolved.class.name)
            {
                continue;
            }

            if analyzed.write(
                plan,
                index,
                if optional {
                    Instruction::PropertyGetOrNullUnchecked {
                        destination,
                        object,
                        slot: PropertySlot::new(resolved.slot),
                    }
                } else {
                    Instruction::PropertyGetUnchecked {
                        destination,
                        object,
                        slot: PropertySlot::new(resolved.slot),
                        value_mode: PropertyReadMode::Clone,
                    }
                },
            ) {
                statistics.property_gets_specialized += 1;
            }
        }
    }
}
