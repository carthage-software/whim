//! Checked compound updates of object properties.

use crate::bytecode::instruction::operands::PropertyRemoveMode;
use crate::vm::Chunk;
use crate::vm::CollectionFault;
use crate::vm::Fault;
use crate::vm::Heap;
use crate::vm::InstanceObject;
use crate::vm::Key;
use crate::vm::ManagedRef;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::append_value;
use crate::vm::arithmetic_add;
use crate::vm::index_get;
use crate::vm::index_replace_existing;
use crate::vm::index_set;
use crate::vm::index_set_reversible;
use crate::vm::name_atom;
use crate::vm::remove_end;
use crate::vm::remove_entry;
use crate::vm::rollback_index_set;
use crate::vm::step_by;
use crate::vm::swap_remove_entry;
use crate::vm::unreachable_invariant;
use crate::vm::vec_append;

struct UpdateFault {
    fault: Fault,
    operator: &'static str,
    right_kind: &'static str,
}

impl VirtualMachine<'_> {
    #[inline(always)]
    fn property_update_receiver(
        &mut self,
        value: Value,
    ) -> Result<ManagedRef<InstanceObject>, VirtualMachineControl> {
        match value.as_object().cloned() {
            Some(receiver) => Ok(receiver),
            None => {
                let kind = value.kind_name();
                Err(self.throw_well_known(
                    self.engine.tables.well_known.type_error,
                    format!("cannot access a property on {kind}"),
                ))
            }
        }
    }

    pub(in crate::vm) fn remove_property(
        &mut self,
        object: Value,
        operand: Option<Value>,
        mode: PropertyRemoveMode,
        chunk: &Chunk,
        site: usize,
    ) -> Result<Value, VirtualMachineControl> {
        let receiver = self.property_update_receiver(object)?;

        let slot = self.property_slot_for(site, chunk, receiver.class())?;

        if receiver.slot_is_uninitialized(slot as usize) {
            let name = name_atom(chunk, site);
            return Err(self.uninitialized_property_error(&receiver, name));
        }

        self.check_readonly_write(&receiver, slot, chunk, site)?;
        receiver
            .mutate_slot(slot as usize, |property| {
                remove_property_value(&self.heap, property, operand.as_ref(), mode)
            })
            .map_err(|fault| self.collection_fault(fault))
    }

    pub(in crate::vm) fn remove_property_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        operand: Option<Value>,
        mode: PropertyRemoveMode,
        slot: u32,
    ) -> Result<Value, VirtualMachineControl> {
        if receiver.slot_is_uninitialized(slot as usize) {
            return Err(self.uninitialized_property_slot_error(receiver, slot as usize));
        }

        receiver
            .mutate_slot(slot as usize, |property| {
                remove_property_value(&self.heap, property, operand.as_ref(), mode)
            })
            .map_err(|fault| self.collection_fault(fault))
    }

    pub(in crate::vm) fn set_property_index(
        &mut self,
        object: Value,
        index: Value,
        value: Value,
        chunk: &Chunk,
        site: usize,
    ) -> Result<(), VirtualMachineControl> {
        let receiver = self.property_update_receiver(object)?;

        let slot = self.property_slot_for(site, chunk, receiver.class())?;

        if receiver.slot_is_uninitialized(slot as usize) {
            let name = name_atom(chunk, site);
            return Err(self.uninitialized_property_error(&receiver, name));
        }

        let preserves_type =
            self.instance_property_index_set_preserves_type(&receiver, slot, &index, &value);
        let rollback = receiver
            .mutate_slot(slot as usize, |property| {
                index_set_reversible(property, &index, value)
            })
            .map_err(|fault| self.collection_fault(fault))?;

        let validation = self.check_readonly_write(&receiver, slot, chunk, site);
        let validation = if validation.is_ok() && !preserves_type {
            let property = receiver.read_slot(slot as usize);
            let validation = self.check_instance_property_value(&receiver, slot, &property);
            drop(property);
            validation
        } else {
            validation
        };

        if let Err(control) = validation {
            receiver.mutate_slot(slot as usize, |property| {
                rollback_index_set(property, rollback);
            });

            return Err(control);
        }

        drop(rollback);
        Ok(())
    }

    pub(in crate::vm) fn set_property_index_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        index: Value,
        value: Value,
        slot: u32,
    ) -> Result<(), VirtualMachineControl> {
        if receiver.slot_is_uninitialized(slot as usize) {
            return Err(self.uninitialized_property_slot_error(receiver, slot as usize));
        }

        receiver
            .mutate_slot(slot as usize, |property| index_set(property, &index, value))
            .map_err(|fault| self.collection_fault(fault))
    }

    pub(in crate::vm) fn append_property(
        &mut self,
        object: Value,
        value: Value,
        chunk: &Chunk,
        site: usize,
    ) -> Result<(), VirtualMachineControl> {
        let receiver = self.property_update_receiver(object)?;

        let slot = self.property_slot_for(site, chunk, receiver.class())?;

        if receiver.slot_is_uninitialized(slot as usize) {
            let name = name_atom(chunk, site);
            return Err(self.uninitialized_property_error(&receiver, name));
        }

        let preserves_type =
            self.instance_property_index_update_preserves_type(&receiver, slot, &value);
        receiver
            .mutate_slot(slot as usize, |property| append_value(property, value))
            .map_err(|fault| self.collection_fault(fault))?;

        let validation = self.check_readonly_write(&receiver, slot, chunk, site);
        let validation = if validation.is_ok() && !preserves_type {
            let property = receiver.read_slot(slot as usize);
            let validation = self.check_instance_property_value(&receiver, slot, &property);
            drop(property);
            validation
        } else {
            validation
        };

        if let Err(control) = validation {
            let rollback =
                receiver.mutate_slot(slot as usize, |property| remove_end(property, false));

            match rollback {
                Ok(value) => drop(value),
                // SAFETY: the surrounding invariant makes this path unreachable.
                Err(_) => unsafe {
                    unreachable_invariant("a property append rolls back its final vec element")
                },
            }

            return Err(control);
        }

        Ok(())
    }

    pub(in crate::vm) fn append_property_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        value: Value,
        slot: u32,
    ) -> Result<(), VirtualMachineControl> {
        if receiver.slot_is_uninitialized(slot as usize) {
            return Err(self.uninitialized_property_slot_error(receiver, slot as usize));
        }

        receiver.mutate_slot(slot as usize, |property| vec_append(property, value));
        Ok(())
    }

    pub(in crate::vm) fn increment_property_index_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        index: Value,
        slot: u32,
    ) -> Result<(), VirtualMachineControl> {
        if receiver.slot_is_uninitialized(slot as usize) {
            return Err(self.uninitialized_property_slot_error(receiver, slot as usize));
        }

        let property = receiver.read_slot(slot as usize);
        let previous = index_get(&self.heap, &property, &index)
            .map_err(|fault| self.collection_fault(fault))?;
        drop(property);
        let incremented = step_by(&previous, 1).map_err(|fault| {
            let kind = previous.kind_name();
            self.binary_fault(fault, "+", kind, "int")
        })?;
        let replaced = receiver
            .mutate_slot(slot as usize, |property| {
                index_replace_existing(property, &index, incremented)
            })
            .map_err(|fault| self.collection_fault(fault))?;
        drop(replaced);
        Ok(())
    }

    pub(in crate::vm) fn fill_property_int_range(
        &mut self,
        object: Value,
        value: Value,
        limit: Value,
        chunk: &Chunk,
        site: usize,
    ) -> Result<(), VirtualMachineControl> {
        let Some(maximum) = limit.as_int() else {
            return Err(self.binary_fault(Fault::Incompatible, "<=", "int", limit.kind_name()));
        };

        if maximum < 0 {
            return Ok(());
        }

        let receiver = self.property_update_receiver(object)?;

        let slot = self.property_slot_for(site, chunk, receiver.class())?;
        let mut updated = receiver.read_slot(slot as usize);
        if updated.is_uninitialized() {
            let name = name_atom(chunk, site);
            return Err(self.uninitialized_property_error(&receiver, name));
        }

        index_set(&mut updated, &Value::int(0), value.clone())
            .map_err(|fault| self.collection_fault(fault))?;
        self.check_readonly_write(&receiver, slot, chunk, site)?;
        self.check_instance_property_value(&receiver, slot, &updated)?;
        drop(receiver.write_slot(slot as usize, updated));

        let mut key = 1;
        while key <= maximum {
            let dictionary_update = receiver.mutate_slot(slot as usize, |property| {
                let dictionary = property.as_dict_mut()?;
                Some(dictionary.make_mut().insert(Key::Int(key), value.clone()))
            });

            if let Some(previous) = dictionary_update {
                let updated = receiver.read_slot(slot as usize);
                let valid = self.check_instance_property_value(&receiver, slot, &updated);
                drop(updated);
                if let Err(control) = valid {
                    receiver.mutate_slot(slot as usize, |property| {
                        let Some(dictionary) = property.as_dict_mut() else {
                            // SAFETY: the surrounding invariant makes this path unreachable.
                            unsafe {
                                unreachable_invariant(
                                    "an indexed dictionary fill retains its container kind",
                                )
                            }
                        };

                        match previous {
                            Some(previous) => {
                                if dictionary
                                    .make_mut()
                                    .insert(Key::Int(key), previous)
                                    .is_none()
                                {
                                    // SAFETY: the surrounding invariant makes this path unreachable.
                                    unsafe {
                                        unreachable_invariant(
                                            "the rollback replaces an existing dictionary key",
                                        )
                                    }
                                }
                            }
                            None => drop(dictionary.make_mut().remove(&Key::Int(key))),
                        }
                    });

                    return Err(control);
                }
            } else {
                let mut updated = receiver.read_slot(slot as usize);
                index_set(&mut updated, &Value::int(key), value.clone())
                    .map_err(|fault| self.collection_fault(fault))?;
                self.check_instance_property_value(&receiver, slot, &updated)?;
                drop(receiver.write_slot(slot as usize, updated));
            }

            key += 1;
        }

        Ok(())
    }

    pub(in crate::vm) fn step_property(
        &mut self,
        object: Value,
        chunk: &Chunk,
        site: usize,
        step: i64,
    ) -> Result<(), VirtualMachineControl> {
        self.update_property(object, chunk, site, |_, previous| {
            step_by(previous, step).map_err(|fault| UpdateFault {
                fault,
                operator: "+",
                right_kind: "int",
            })
        })
    }

    pub(in crate::vm) fn step_property_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        step: i64,
    ) -> Result<(), VirtualMachineControl> {
        self.update_property_unchecked(receiver, slot, |_, previous| {
            step_by(previous, step).map_err(|fault| UpdateFault {
                fault,
                operator: "+",
                right_kind: "int",
            })
        })
    }

    pub(in crate::vm) fn add_to_property(
        &mut self,
        object: Value,
        source: Value,
        chunk: &Chunk,
        site: usize,
    ) -> Result<(), VirtualMachineControl> {
        let right_kind = source.kind_name();
        self.update_property(object, chunk, site, |heap, previous| {
            arithmetic_add(heap, previous, &source).map_err(|fault| UpdateFault {
                fault,
                operator: "+",
                right_kind,
            })
        })
    }

    pub(in crate::vm) fn add_to_property_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        source: Value,
        slot: u32,
    ) -> Result<(), VirtualMachineControl> {
        let right_kind = source.kind_name();
        self.update_property_unchecked(receiver, slot, |heap, previous| {
            arithmetic_add(heap, previous, &source).map_err(|fault| UpdateFault {
                fault,
                operator: "+",
                right_kind,
            })
        })
    }

    fn update_property(
        &mut self,
        object: Value,
        chunk: &Chunk,
        site: usize,
        operation: impl FnOnce(&Heap, &Value) -> Result<Value, UpdateFault>,
    ) -> Result<(), VirtualMachineControl> {
        let receiver = self.property_update_receiver(object)?;

        let slot = self.property_slot_for(site, chunk, receiver.class())?;
        let previous = receiver.read_slot(slot as usize);
        if previous.is_uninitialized() {
            let name = name_atom(chunk, site);
            return Err(self.uninitialized_property_error(&receiver, name));
        }

        let updated = match operation(&self.heap, &previous) {
            Ok(updated) => updated,
            Err(UpdateFault {
                fault,
                operator,
                right_kind,
            }) => {
                let left_kind = previous.kind_name();
                return Err(self.binary_fault(fault, operator, left_kind, right_kind));
            }
        };

        self.check_readonly_write(&receiver, slot, chunk, site)?;
        self.check_instance_property_value(&receiver, slot, &updated)?;
        drop(receiver.write_slot(slot as usize, updated));
        Ok(())
    }

    fn update_property_unchecked(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: u32,
        operation: impl FnOnce(&Heap, &Value) -> Result<Value, UpdateFault>,
    ) -> Result<(), VirtualMachineControl> {
        let previous = receiver.read_slot(slot as usize);
        if previous.is_uninitialized() {
            return Err(self.uninitialized_property_slot_error(receiver, slot as usize));
        }

        let updated = match operation(&self.heap, &previous) {
            Ok(updated) => updated,
            Err(UpdateFault {
                fault,
                operator,
                right_kind,
            }) => {
                let left_kind = previous.kind_name();
                return Err(self.binary_fault(fault, operator, left_kind, right_kind));
            }
        };

        drop(receiver.write_slot(slot as usize, updated));
        Ok(())
    }
}

fn remove_property_value(
    heap: &Heap,
    property: &mut Value,
    operand: Option<&Value>,
    mode: PropertyRemoveMode,
) -> Result<Value, CollectionFault> {
    match mode {
        PropertyRemoveMode::Key => {
            let Some(key) = operand else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a keyed property removal receives its key") }
            };
            remove_entry(heap, property, key)
        }
        PropertyRemoveMode::Swap => {
            let Some(index) = operand else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("a swap removal receives its index") }
            };
            swap_remove_entry(property, index)
        }
        PropertyRemoveMode::First => remove_end(property, true),
        PropertyRemoveMode::Last => remove_end(property, false),
    }
}
