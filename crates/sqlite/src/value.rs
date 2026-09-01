/// A parameter or result value.
#[derive(Clone, Debug)]
pub enum Value {
    /// SQL `NULL`.
    Null,
    /// A signed integer.
    Integer(i64),
    /// A floating-point value.
    Real(f64),
    /// UTF-8 text bytes.
    Text(Vec<u8>),
    /// Binary bytes.
    Blob(Vec<u8>),
}

/// One result row.
pub type Row = Vec<Value>;

/// Metadata for one result column.
#[derive(Clone, Debug)]
pub struct Column {
    /// The column name.
    pub name: Vec<u8>,
    /// The declared database type.
    pub declared_type: Option<Vec<u8>>,
}

/// Metadata for a query result.
#[derive(Clone, Debug)]
pub struct Metadata {
    /// The result columns.
    pub columns: Vec<Column>,
    /// The affected row count for a command result.
    pub affected_rows: Option<u64>,
}
