//! Compile errors: the static gates and their reporting.

use whim_span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompileErrorKind {
    StandaloneWildcardType,
    WildcardTypeArgument,
    MemberWithoutVisibility,
    IntegerLiteralOutOfRange,
    TryWithoutClause,
    ReturnInsideFinally,
    ReturnOutsideCallable,
    LoopJumpEscapesFinally,
    ValueReturnInVoidFunction,
    ReturnInNeverFunction,
    VoidExpressionShortClosure,
    VoidInUnion,
    ReturnOnlyType,
    AliasOfVoid,
    TypeNotRuntimeCheckable,
    InvalidClassnameType,
    TooManyArguments,
    TooManyRegisters,
    SideTableFull,
    DuplicateParameter,
    DuplicateUsingBinding,
    DuplicateNamedArgument,
    PositionalArgumentAfterNamedArgument,
    ThisOutsideMethod,
    CannotDropThis,
    CannotBindThis,
    CannotAssignFinalLocal,
    ClassContextRequired,
    InvalidMemberType,
    LoopJumpOutsideLoop,
    DynamicClassMemberAccess,
    InvalidIncrementTarget,
    InvalidCompoundAssignmentTarget,
    AppendTargetUsedAsValue,
    NestedDeclaration,
    ClassConstantTypeMismatch,
    InvalidEnumBacking,
    EnumCaseValueMismatch,
    RedundantTypeComposition,
    MultipleBaseClasses,
    AbstractMethodInConcreteClass,
    UnreachableMatchArm,
    InconsistentPatternBindings,
    DuplicatePatternBinding,
    DuplicateDictionaryKey,
    DuplicateImportAlias,
    GenericEnum,
    TypeArgumentArityMismatch,
    TypeArgumentBoundViolation,
    NonTrailingTypeParameterDefault,
    UnboundTypeParameterDefault,
    ClassTypeParameterInStaticMember,
    TypeParameterClassReference,
    InvalidVarianceUse,
    RecursiveTypeAlias,
    DuplicateModifier,
    ConflictingModifiers,
    ModifierNotAllowed,
    InvalidStaticOnlyClassMember,
    AbstractBodyMismatch,
    MemberNotAllowed,
    InvalidInterfaceProperty,
    DuplicateMember,
    EnumCaseValueMissing,
    EnumCaseValueNotAllowed,
    DuplicateEnumCaseValue,
    EnumBuiltInMethodRedeclaration,
    SealedPermissionViolation,
    ParameterModifierOutsideConstructor,
    InvalidLifecycleMethod,
    DuplicateCapture,
    NonConstantAttributeArgument,
    NonConstantParameterDefault,
    NonConstantInitializer,
    NonConstantPropertyDefault,
    EmbeddedFileRequiresPath,
    AbsoluteEmbeddedFilePath,
    ReadEmbeddedFile,
    TooManyTypeCompositionMembers,
    TooManyTypeArguments,
    TooManyParameters,
    TooManyTypeParameters,
    DuplicateTypeParameter,
    TooManyCaptures,
    TooManyInterfaces,
    TooManyMembers,
    TooManyTupleElements,
    SpreadInTuple,
    TargetAfterRest,
    RequiredTargetAfterDefault,
    TooManyCatchClauses,
    TooManyAttributes,
    TooManyWrittenValues,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompileError {
    pub(crate) message: String,
    pub(crate) span: Span,
    pub(crate) kind: CompileErrorKind,
    pub(crate) notes: Vec<(Span, String)>,
}

impl CompileError {
    #[must_use]
    pub(crate) fn new(kind: CompileErrorKind, message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            kind,
            notes: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_note(mut self, span: Span, message: impl Into<String>) -> Self {
        self.notes.push((span, message.into()));
        self
    }

    #[must_use]
    pub(crate) fn labels(&self) -> Vec<(Span, &str)> {
        let mut labels = vec![(self.span, self.message.as_str())];
        labels.extend(
            self.notes
                .iter()
                .map(|(span, message)| (*span, message.as_str())),
        );
        labels
    }
}
