//! Loading precompiled Whim artifacts into an engine.

use std::rc::Rc;

use crate::artifact::DecodedArtifact;
use crate::artifact::decode;
use crate::artifact::decode_static;
use crate::bytecode::verify::verify_unit;
use crate::engine::Engine;
use crate::engine::EngineError;
use crate::symbols::UnitSourceFile;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;

impl Engine {
    /// Verifies, declares, and executes a previously compiled artifact.
    ///
    /// # Errors
    ///
    /// Returns an error when decoding, verification, declaration, or top-level
    /// execution fails.
    pub fn load_artifact(&mut self, artifact: &[u8]) -> Result<(), EngineError> {
        let mut decoded = self.decode_artifact(artifact)?;
        self.expand_declared_types(&mut decoded.unit);
        if let Err(error) = verify_unit(&decoded.unit) {
            return Err(
                self.engine_error_message(format!("artifact bytecode did not verify: {error:?}"))
            );
        }

        self.initialize_artifact(decoded)
    }

    /// Declares and executes a previously verified artifact.
    ///
    /// # Safety
    ///
    /// `artifact` must be the unchanged output of [`Engine::compile_artifact`]
    /// from this build of `whim-runtime`. Unverified bytecode may violate VM
    /// safety invariants.
    #[doc(hidden)]
    pub unsafe fn load_verified_artifact(&mut self, artifact: &[u8]) -> Result<(), EngineError> {
        let mut decoded = self.decode_artifact(artifact)?;
        self.expand_declared_types(&mut decoded.unit);
        self.initialize_artifact(decoded)
    }

    /// Declares and executes a statically retained, previously verified artifact.
    ///
    /// # Safety
    ///
    /// `artifact` must be the unchanged output of [`Engine::compile_artifact`]
    /// from this build of `whim-runtime`.
    #[doc(hidden)]
    pub unsafe fn load_verified_static_artifact(
        &mut self,
        artifact: &'static [u8],
    ) -> Result<(), EngineError> {
        let decoded = decode_static(artifact, &self.heap)
            .map_err(|error| self.engine_error_message(error.to_string()))?;
        self.initialize_artifact(decoded)
    }

    fn decode_artifact(&self, artifact: &[u8]) -> Result<DecodedArtifact, EngineError> {
        decode(artifact, &self.heap).map_err(|error| self.engine_error_message(error.to_string()))
    }

    fn initialize_artifact(&mut self, decoded: DecodedArtifact) -> Result<(), EngineError> {
        let unit = decoded.unit;

        let mut source_files = Vec::with_capacity(decoded.source_files.len());
        for file in decoded.source_files {
            source_files.push(UnitSourceFile {
                path: self.heap.intern(file.path.as_bytes()),
                start: file.start,
                end: file.end,
                line_starts: file.line_starts,
            });
        }

        let unit = Rc::new(unit);
        let context = match self.declare_compiled_with_source_files(
            &unit,
            decoded.line_starts,
            decoded.source,
            source_files,
        ) {
            Ok(context) => context,
            Err(VirtualMachineControl::Throw(value)) => return Err(self.engine_error(value)),
            Err(VirtualMachineControl::Exit(code)) => {
                return Err(self.engine_error_message(format!(
                    "artifact initialization exited with code {code}"
                )));
            }
        };

        let mut vm = VirtualMachine::new(self);
        let outcome = vm.run_main(&context);
        vm.clear_main_task_local_values();
        drop(vm);
        match outcome {
            Ok(value) => {
                drop(value);
                Ok(())
            }
            Err(VirtualMachineControl::Throw(value)) => Err(self.engine_error(value)),
            Err(VirtualMachineControl::Exit(code)) => Err(self
                .engine_error_message(format!("artifact initialization exited with code {code}"))),
        }
    }
}
