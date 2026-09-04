//! Compilation and encoding of loadable Whim artifacts.

#![deny(clippy::nursery, clippy::pedantic)]

use std::error::Error;
use std::fmt;
use std::rc::Rc;
use std::str::from_utf8;
use std::str::from_utf8_unchecked;

use bincode::serialize;
use whim_span::Position;
use whim_syn::arena::Arena;
use whim_syn::arena::LocalArena;
use whim_syn::parser;

use crate::bytecode::aliases::expand_unit_declarations;
use crate::bytecode::decode::compiled_unit;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify_unit;
use crate::compiler::AliasGraph;
use crate::compiler::Compilation;
use crate::compiler::CompilePath;
use crate::compiler::EmbeddedFiles;
use crate::compiler::GenericTable;
use crate::compiler::compile_program_into_unit;
use crate::compiler::extend_generics;
use crate::compiler::finish_unit;
use crate::compiler::new_unit;
use crate::engine::Engine;
use crate::optimizer::OptimizationConfiguration;
use crate::symbols::SourceText;
use crate::symbols::UnitSourceFile;
use crate::symbols::line_starts_of;
use crate::value::heap::Heap;
use crate::vm::VirtualMachineControl;

const MAGIC: &[u8; 8] = b"WHIM\0\0\0\0";
const FORMAT_VERSION: u32 = 6;

mod merge;

/// One source file compiled into an artifact.
#[derive(Debug, Clone, Copy)]
pub struct SourceFile<'source> {
    path: &'source str,
    contents: &'source str,
}

impl<'source> SourceFile<'source> {
    /// Creates a source file from its diagnostic path and contents.
    #[must_use]
    pub const fn new(path: &'source str, contents: &'source str) -> Self {
        Self { path, contents }
    }
}

/// Options controlling artifact compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactConfiguration {
    /// Whether bytecode optimization is enabled.
    pub optimize: bool,
    /// Whether written return types are trusted and need no runtime check.
    pub trusted_return_types: bool,
}

impl Default for ArtifactConfiguration {
    fn default() -> Self {
        Self {
            optimize: true,
            trusted_return_types: false,
        }
    }
}

/// A versioned, loadable collection of compiled Whim source files.
pub struct Artifact {
    bytes: Vec<u8>,
}

impl Artifact {
    /// Consumes the artifact and returns its encoded bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// A failure while compiling or encoding an artifact.
#[derive(Debug)]
pub struct ArtifactError {
    message: String,
}

impl ArtifactError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ArtifactError {}

pub(crate) struct DecodedSourceFile {
    pub(crate) path: String,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) line_starts: Vec<u32>,
}

pub(crate) struct DecodedArtifact {
    pub(crate) unit: CompiledUnit,
    pub(crate) source: SourceText,
    pub(crate) line_starts: Vec<u32>,
    pub(crate) source_files: Vec<DecodedSourceFile>,
}

impl Engine {
    /// Compiles source files together into one optimized, verified artifact.
    ///
    /// # Errors
    ///
    /// Returns an error when parsing, compilation, linking, verification, or
    /// encoding fails.
    pub fn compile_artifact(
        &mut self,
        path: &str,
        sources: &[SourceFile<'_>],
        configuration: ArtifactConfiguration,
    ) -> Result<Artifact, ArtifactError> {
        let (source, source_files) = join_sources(sources)?;
        let line_starts = line_starts_of(&source);

        let arena = LocalArena::new();
        let parsed_source = arena.alloc_str(&source);
        let mut programs = Vec::with_capacity(sources.len());
        for (file, retained) in sources.iter().zip(&source_files) {
            let contents = &parsed_source[retained.start as usize..retained.end as usize];
            let program = parser::parse_fragment(
                &arena,
                parsed_source,
                contents,
                Position::new(retained.start),
            )
            .map_err(|errors| {
                ArtifactError::new(format!("failed to parse {}:\n{errors}", file.path))
            })?;
            programs.push(program);
        }

        let mut generics = GenericTable::new();
        for program in &programs {
            extend_generics(program, &mut generics);
        }

        let mut unit = new_unit(path.as_bytes(), &self.heap);
        let mut aliases = AliasGraph::default();
        let embedded_files = EmbeddedFiles::default();
        let mut compilation = Compilation::new(
            &generics,
            &mut aliases,
            &embedded_files,
            &line_starts,
            configuration.trusted_return_types,
        );
        let mut main_chunks = Vec::with_capacity(programs.len());
        for (file, program) in sources.iter().zip(&programs) {
            let chunk = compile_program_into_unit(
                &self.heap,
                program,
                CompilePath {
                    diagnostic: file.path,
                    runtime: file.path.as_bytes(),
                },
                &mut unit,
                &mut compilation,
            )
            .map_err(|error| {
                ArtifactError::new(format!(
                    "failed to compile {}:\n{}",
                    file.path, error.message
                ))
            })?;
            main_chunks.push(chunk);
        }

        unit.main = merge::main(main_chunks)?;
        let mut unit = finish_unit(
            unit,
            &aliases,
            &self.heap,
            &self.tables.built_in_function_declarations,
            OptimizationConfiguration {
                enabled: configuration.optimize,
                ..OptimizationConfiguration::default()
            },
        )
        .map_err(|error| {
            let source_path =
                source_path_for_span(&source_files, error.span.start.offset).unwrap_or(path);
            ArtifactError::new(format!(
                "failed to compile {source_path}:\n{}",
                error.message
            ))
        })?;

        let aliases = unit.type_aliases.clone();
        expand_unit_declarations(&mut unit, &aliases);
        verify_artifact_unit(&unit, "compiled")?;

        let retained_source = Rc::<str>::from(source.as_str());
        let retained_files = retain_source_files(&self.heap, &source_files);
        let unit = match self.prepare_artifact_unit(
            unit,
            line_starts.clone(),
            retained_source,
            retained_files,
        ) {
            Ok(unit) => unit,
            Err(VirtualMachineControl::Throw(value)) => {
                let error = self.engine_error(value);
                return Err(ArtifactError::new(format!(
                    "failed to link {path}:\n{}",
                    error.rendered()
                )));
            }
            Err(VirtualMachineControl::Exit(code)) => {
                return Err(ArtifactError::new(format!(
                    "failed to link {path}: validation exited with code {code}"
                )));
            }
        };
        verify_artifact_unit(&unit, "linked")?;

        encode(&unit, &source, &line_starts, &source_files)
    }
}

fn retain_source_files(heap: &Heap, files: &[DecodedSourceFile]) -> Vec<UnitSourceFile> {
    files
        .iter()
        .map(|file| UnitSourceFile {
            path: heap.intern(file.path.as_bytes()),
            start: file.start,
            end: file.end,
            line_starts: file.line_starts.clone(),
        })
        .collect()
}

fn join_sources(
    sources: &[SourceFile<'_>],
) -> Result<(String, Vec<DecodedSourceFile>), ArtifactError> {
    let mut source = String::new();
    let mut source_files = Vec::with_capacity(sources.len());
    for file in sources {
        let start = u32::try_from(source.len())
            .map_err(|_| ArtifactError::new("artifact source exceeds the format limit"))?;
        source.push_str(file.contents);
        let end = u32::try_from(source.len())
            .map_err(|_| ArtifactError::new("artifact source exceeds the format limit"))?;
        source.push_str("\n\n");
        source_files.push(DecodedSourceFile {
            path: file.path.to_string(),
            start,
            end,
            line_starts: line_starts_of(file.contents),
        });
    }

    Ok((source, source_files))
}

fn source_path_for_span(files: &[DecodedSourceFile], offset: u32) -> Option<&str> {
    files
        .iter()
        .find(|file| file.start <= offset && offset <= file.end)
        .map(|file| file.path.as_str())
}

fn encode(
    unit: &CompiledUnit,
    source: &str,
    line_starts: &[u32],
    source_files: &[DecodedSourceFile],
) -> Result<Artifact, ArtifactError> {
    let bytecode = serialize(&unit)
        .map_err(|error| ArtifactError::new(format!("failed to encode bytecode: {error}")))?;
    let source_length = u64::try_from(source.len())
        .map_err(|_| ArtifactError::new("artifact source exceeds the format limit"))?;
    let bytecode_length = u64::try_from(bytecode.len())
        .map_err(|_| ArtifactError::new("artifact bytecode exceeds the format limit"))?;
    let source_file_count = u32::try_from(source_files.len())
        .map_err(|_| ArtifactError::new("artifact has too many source files"))?;
    let line_start_count = u32::try_from(line_starts.len())
        .map_err(|_| ArtifactError::new("artifact has too many source lines"))?;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    push_u32(&mut bytes, FORMAT_VERSION);
    push_u64(&mut bytes, source_length);
    push_u64(&mut bytes, bytecode_length);
    push_u32(&mut bytes, source_file_count);
    push_u32(&mut bytes, line_start_count);
    for start in line_starts {
        push_u32(&mut bytes, *start);
    }
    for file in source_files {
        let path = file.path.as_bytes();
        let path_length = u32::try_from(path.len())
            .map_err(|_| ArtifactError::new("artifact source path exceeds the format limit"))?;
        push_u32(&mut bytes, path_length);
        bytes.extend_from_slice(path);
        push_u32(&mut bytes, file.start);
        push_u32(&mut bytes, file.end);
        let line_start_count = u32::try_from(file.line_starts.len())
            .map_err(|_| ArtifactError::new("artifact source file has too many lines"))?;
        push_u32(&mut bytes, line_start_count);
        for start in &file.line_starts {
            push_u32(&mut bytes, *start);
        }
    }
    bytes.extend_from_slice(source.as_bytes());
    bytes.extend_from_slice(&bytecode);

    Ok(Artifact { bytes })
}

struct DecodedParts<'source> {
    unit: CompiledUnit,
    source: &'source str,
    line_starts: Vec<u32>,
    source_files: Vec<DecodedSourceFile>,
}

pub(crate) fn decode(bytes: &[u8], heap: &Heap) -> Result<DecodedArtifact, ArtifactError> {
    let decoded = decode_parts(bytes, heap, false)?;
    Ok(DecodedArtifact {
        unit: decoded.unit,
        source: SourceText::Shared(Rc::from(decoded.source)),
        line_starts: decoded.line_starts,
        source_files: decoded.source_files,
    })
}

pub(crate) fn decode_static(
    bytes: &'static [u8],
    heap: &Heap,
) -> Result<DecodedArtifact, ArtifactError> {
    let decoded = decode_parts(bytes, heap, true)?;
    Ok(DecodedArtifact {
        unit: decoded.unit,
        source: SourceText::Static(decoded.source),
        line_starts: decoded.line_starts,
        source_files: decoded.source_files,
    })
}

fn decode_parts<'source>(
    bytes: &'source [u8],
    heap: &Heap,
    trusted: bool,
) -> Result<DecodedParts<'source>, ArtifactError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(MAGIC.len())? != MAGIC {
        return Err(ArtifactError::new("artifact has an invalid magic header"));
    }

    let version = cursor.u32()?;
    if version != FORMAT_VERSION {
        return Err(ArtifactError::new(format!(
            "artifact format version {version} is unsupported"
        )));
    }

    let source_length = cursor.usize_from_u64("artifact source length")?;
    let bytecode_length = cursor.usize_from_u64("artifact bytecode length")?;
    let source_file_count = cursor.u32()? as usize;
    let line_start_count = cursor.u32()? as usize;

    let line_start_bytes = line_start_count
        .checked_mul(4)
        .ok_or_else(|| ArtifactError::new("artifact line count overflows"))?;
    if line_start_bytes > cursor.remaining() {
        return Err(ArtifactError::new(
            "artifact line count exceeds its metadata",
        ));
    }
    let mut line_starts = Vec::with_capacity(line_start_count);
    for _ in 0..line_start_count {
        line_starts.push(cursor.u32()?);
    }

    let source_file_metadata_length = source_file_count
        .checked_mul(16)
        .ok_or_else(|| ArtifactError::new("artifact source-file count overflows"))?;
    if source_file_metadata_length > cursor.remaining() {
        return Err(ArtifactError::new(
            "artifact source-file count exceeds its metadata",
        ));
    }

    let mut source_files = Vec::with_capacity(source_file_count);
    for _ in 0..source_file_count {
        let path_length = cursor.u32()? as usize;
        let path = decode_text(
            cursor.take(path_length)?,
            trusted,
            "artifact contains a non-UTF-8 source path",
        )?
        .to_string();
        let start = cursor.u32()?;
        let end = cursor.u32()?;
        let line_start_count = cursor.u32()? as usize;
        let line_start_bytes = line_start_count
            .checked_mul(4)
            .ok_or_else(|| ArtifactError::new("artifact source-file line count overflows"))?;
        if line_start_bytes > cursor.remaining() {
            return Err(ArtifactError::new(
                "artifact source-file line count exceeds its metadata",
            ));
        }
        let mut file_line_starts = Vec::with_capacity(line_start_count);
        for _ in 0..line_start_count {
            file_line_starts.push(cursor.u32()?);
        }
        source_files.push(DecodedSourceFile {
            path,
            start,
            end,
            line_starts: file_line_starts,
        });
    }

    let source = decode_text(
        cursor.take(source_length)?,
        trusted,
        "artifact contains non-UTF-8 source text",
    )?;
    if !trusted {
        validate_source_metadata(source, &line_starts, &source_files)?;
    }

    let bytecode = cursor.take(bytecode_length)?;
    if !cursor.is_empty() {
        return Err(ArtifactError::new("artifact contains trailing bytes"));
    }

    let unit = compiled_unit(bytecode, heap)
        .map_err(|error| ArtifactError::new(format!("artifact bytecode is invalid: {error}")))?;
    Ok(DecodedParts {
        unit,
        source,
        line_starts,
        source_files,
    })
}

fn validate_source_metadata(
    source: &str,
    line_starts: &[u32],
    source_files: &[DecodedSourceFile],
) -> Result<(), ArtifactError> {
    if !valid_line_starts(line_starts, source.len()) {
        return Err(ArtifactError::new(
            "artifact contains invalid source line metadata",
        ));
    }
    for file in source_files {
        if file.start > file.end || source.get(file.start as usize..file.end as usize).is_none() {
            return Err(ArtifactError::new(
                "artifact contains an invalid source-file span",
            ));
        }
        let length = (file.end - file.start) as usize;
        if !valid_line_starts(&file.line_starts, length) {
            return Err(ArtifactError::new(
                "artifact contains invalid source-file line metadata",
            ));
        }
    }

    Ok(())
}

fn decode_text<'bytes>(
    bytes: &'bytes [u8],
    trusted: bool,
    error: &'static str,
) -> Result<&'bytes str, ArtifactError> {
    if trusted {
        // SAFETY: only the build-verified embedded artifact uses this path.
        return Ok(unsafe { from_utf8_unchecked(bytes) });
    }

    from_utf8(bytes).map_err(|_| ArtifactError::new(error))
}

fn valid_line_starts(starts: &[u32], source_length: usize) -> bool {
    starts.first() == Some(&0)
        && starts.windows(2).all(|pair| pair[0] < pair[1])
        && starts
            .last()
            .is_some_and(|last| *last as usize <= source_length)
}

fn verify_artifact_unit(unit: &CompiledUnit, stage: &str) -> Result<(), ArtifactError> {
    verify_unit(unit)
        .map_err(|error| ArtifactError::new(format!("{stage} artifact did not verify: {error:?}")))
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'bytes> {
    bytes: &'bytes [u8],
    position: usize,
}

impl<'bytes> Cursor<'bytes> {
    const fn new(bytes: &'bytes [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'bytes [u8], ArtifactError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| ArtifactError::new("artifact section length overflows"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| ArtifactError::new("artifact ends before its declared sections"))?;
        self.position = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, ArtifactError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| ArtifactError::new("artifact contains an incomplete integer"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn usize_from_u64(&mut self, name: &str) -> Result<usize, ArtifactError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| ArtifactError::new("artifact contains an incomplete integer"))?;
        usize::try_from(u64::from_le_bytes(bytes))
            .map_err(|_| ArtifactError::new(format!("{name} exceeds this platform's limit")))
    }

    const fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}
