/// A query result value.
#[derive(Clone, Debug)]
pub enum Value {
    /// SQL `NULL`.
    Null,
    /// A Boolean value.
    Boolean(bool),
    /// A signed integer.
    Integer(i64),
    /// A floating-point value.
    Real(f64),
    /// Text bytes.
    Text(Vec<u8>),
    /// Binary bytes.
    Blob(Vec<u8>),
}

/// A borrowed query parameter.
#[derive(Clone, Copy, Debug)]
pub enum Parameter<'value> {
    /// SQL `NULL`.
    Null,
    /// A Boolean value.
    Boolean(bool),
    /// A signed integer.
    Integer(i64),
    /// A floating-point value.
    Real(f64),
    /// Text bytes.
    Text(&'value [u8]),
    /// Binary bytes.
    Blob(&'value [u8]),
}

/// One result row.
pub type Row = Vec<Value>;

/// Metadata for one result column.
#[derive(Clone, Debug)]
pub struct Column {
    /// The column name.
    pub name: Vec<u8>,
    /// The known database type name.
    pub type_name: Option<Vec<u8>>,
}

/// Metadata for a query result.
#[derive(Clone, Debug)]
pub struct Metadata {
    /// The result columns.
    pub columns: Vec<Column>,
    /// The affected row count for a command result.
    pub affected_rows: Option<u64>,
}
