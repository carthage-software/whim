#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Category {
    Clarity,
    BestPractices,
    Correctness,
    Maintainability,
    Redundancy,
    Security,
}

impl Category {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clarity => "Clarity",
            Self::BestPractices => "Best Practices",
            Self::Correctness => "Correctness",
            Self::Maintainability => "Maintainability",
            Self::Redundancy => "Redundancy",
            Self::Security => "Security",
        }
    }
}
