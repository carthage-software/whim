//! Register-access and instruction-dispatch macros shared by the executor
//! and the numeric-loop interpreter.

macro_rules! read_register {
    ($registers:expr, $register:expr) => {
        // SAFETY: verified bytecode keeps the register in the active frame.
        unsafe { (*$registers.add($register.index() as usize)).clone_inline_scalar() }
    };
}

/// Evaluates the value before writing it, so it may read the register it replaces.
macro_rules! write_register {
    ($registers:expr, $register:expr, $value:expr) => {{
        let value = $value;
        // SAFETY: verified bytecode keeps the register in the active frame.
        unsafe {
            let destination = $registers.add($register.index() as usize);
            if !(&*destination).is_reference_counted() {
                destination.write(value);
            } else {
                *destination = value;
            }
        }
    }};
}

/// Requires the destination register to already hold an int.
macro_rules! write_proven_int_register {
    ($registers:expr, $register:expr, $value:expr) => {{
        // SAFETY: verified bytecode keeps the register in the active frame.
        let destination = unsafe { $registers.add($register.index() as usize) };
        // SAFETY: `destination` points to an initialized active-frame value.
        debug_assert!(unsafe { &*destination }.is_int());
        // SAFETY: type flow proves the old value is an inline int, so it needs no drop.
        unsafe { destination.write(Value::int($value)) };
    }};
}

/// Requires tag dispatch to have already selected the variant.
macro_rules! decode_instruction {
    ($word:ident, $variant:ident, { $($fields:tt)* }) => {
        // SAFETY: dispatch matched the word's instruction tag.
        let instruction = unsafe { $word.decode() };
        let Instruction::$variant { $($fields)* } = instruction else {
            // SAFETY: `decode` must return the variant selected by the tag.
            unsafe { unreachable_invariant("an instruction tag selects its own payload") }
        };
    };
    ($word:ident, $variant:ident) => {};
}

macro_rules! dispatch_instruction {
    (
        $word:ident {
            $(
                Instruction::$variant:ident $( { $($fields:tt)* } )? => $body:block
            )*
            _ => $fallback:block
        }
    ) => {
        match $word.kind() {
            $(
                InstructionKind::$variant => {
                    decode_instruction!($word, $variant $(, { $($fields)* })?);
                    $body
                }
            )*
            _ => $fallback,
        }
    };
    (
        $word:ident {
            $(
                Instruction::$variant:ident $( { $($fields:tt)* } )? => $body:block
            )*
        }
    ) => {
        match $word.kind() {
            $(
                InstructionKind::$variant => {
                    decode_instruction!($word, $variant $(, { $($fields)* })?);
                    $body
                }
            )*
        }
    };
}

macro_rules! fail {
    ($self:ident, $ip:ident, $floor:ident, $dispatch:lifetime, $control:expr) => {{
        $self.sync_ip($ip);
        let control = $control;
        $self.handle_control(control, $floor)?;
        continue $dispatch;
    }};
}

/// Call after a bytecode call or return to reload the frame without
/// round-tripping through the outer loop.
macro_rules! reload_frame {
    ($self:ident, $chunk:ident, $code:ident, $ip:ident, $registers:ident) => {{
        let (chunk_pointer, next_ip, base) = {
            let frame = $self.current_frame();
            (frame.chunk, frame.ip as usize, frame.base as usize)
        };
        // SAFETY: the current frame owns `chunk_pointer` and its stack window.
        $chunk = unsafe { chunk_pointer.as_ref() };
        $code = $chunk.code.as_ptr();
        $ip = next_ip;
        // SAFETY: the frame base lies within the live VM stack.
        $registers = unsafe { $self.stack.as_mut_ptr().add(base) };
    }};
}

macro_rules! refine_live_tail {
    ($self:ident, $chunk:ident, $code:ident, $ip:ident, $registers:ident, $floor:ident,
     $dispatch:lifetime, $instructions:lifetime) => {{
        if $self.world_refinement_pending && $self.current_frame().function.get().is_none() {
            let marker = $ip - 1;
            $self.sync_ip(marker);
            match $self.refine_current_main_tail(marker) {
                Ok(true) => {
                    reload_frame!($self, $chunk, $code, $ip, $registers);
                    continue $instructions;
                }
                Ok(false) => {}
                Err(control) => fail!($self, $ip, $floor, $dispatch, control),
            }
        }
    }};
}

macro_rules! resume_numeric_loop {
    ($self:ident, $chunk:ident, $registers:ident, $ip:ident, $exit:ident,
     $floor:ident, $dispatch:lifetime, $prepared:literal, $float_registers:expr,
     $dirty_registers:expr, $site:expr) => {{
        // SAFETY: verified numeric-loop bytecode defines the register window and exits.
        match unsafe {
            $self.enter_numeric_region::<$prepared>(
                $chunk,
                $registers,
                $ip,
                $exit,
                $float_registers,
                $dirty_registers,
                $site,
            )
        } {
            NumericLoopTransition::Next(next) => $ip = next,
            NumericLoopTransition::Control { resume_ip, control } => {
                $ip = resume_ip;
                fail!($self, $ip, $floor, $dispatch, control);
            }
        }
    }};
}

macro_rules! binary_arithmetic {
    ($self:ident, $registers:ident, $ip:ident, $floor:ident, $dispatch:lifetime,
     $destination:ident, $left:ident, $right:ident, $operation:path, $operator:literal) => {{
        let outcome = {
            // SAFETY: verified bytecode keeps both registers in the active frame.
            let left_value = unsafe { &*$registers.add($left.index() as usize) };
            // SAFETY: verified bytecode keeps both registers in the active frame.
            let right_value = unsafe { &*$registers.add($right.index() as usize) };
            $operation(&$self.heap, left_value, right_value)
        };
        match outcome {
            Ok(value) => write_register!($registers, $destination, value),
            Err(fault) => {
                // SAFETY: verified bytecode keeps both registers in the active frame.
                let (left_kind, right_kind) = unsafe {
                    (
                        (*$registers.add($left.index() as usize)).kind_name(),
                        (*$registers.add($right.index() as usize)).kind_name(),
                    )
                };
                fail!(
                    $self,
                    $ip,
                    $floor,
                    $dispatch,
                    $self.binary_fault(fault, $operator, left_kind, right_kind)
                );
            }
        }
    }};
}

/// Requires both operands to already be proven ints.
macro_rules! integer_arithmetic {
    ($self:ident, $registers:ident, $ip:ident, $floor:ident, $dispatch:lifetime,
     $destination:ident, $left:ident, $right:ident, $operation:path, $operator:literal) => {{
        // SAFETY: type flow proves both active-frame registers contain ints.
        let left_value = unsafe { int_register($registers, $left) };
        // SAFETY: type flow proves both active-frame registers contain ints.
        let right_value = unsafe { int_register($registers, $right) };
        match $operation(left_value, right_value) {
            Ok(value) => {
                write_register!($registers, $destination, Value::int(value));
            }
            Err(fault) => {
                fail!(
                    $self,
                    $ip,
                    $floor,
                    $dispatch,
                    $self.binary_fault(fault, $operator, "int", "int")
                );
            }
        }
    }};
}
