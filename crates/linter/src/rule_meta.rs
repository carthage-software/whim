use crate::category::Category;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RuleMeta {
    pub name: &'static str,
    pub code: &'static str,
    pub description: &'static str,
    pub good_example: &'static str,
    pub bad_example: &'static str,
    pub category: Category,
}
