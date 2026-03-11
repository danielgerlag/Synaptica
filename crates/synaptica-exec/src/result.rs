use std::fmt;
use synaptica_core::types::Value;

/// A single record (row) produced by query execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub columns: Vec<String>,
    pub values: Vec<Value>,
}

impl Record {
    pub fn new(columns: Vec<String>, values: Vec<Value>) -> Self {
        Self { columns, values }
    }

    /// Look up a value by column name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.columns
            .iter()
            .position(|c| c == name)
            .map(|i| &self.values[i])
    }
}

/// A set of records returned from query execution.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub records: Vec<Record>,
}

impl ResultSet {
    pub fn new(columns: Vec<String>) -> Self {
        Self {
            columns,
            records: Vec::new(),
        }
    }

    pub fn add_record(&mut self, values: Vec<Value>) {
        debug_assert_eq!(
            values.len(),
            self.columns.len(),
            "record has {} values but {} columns",
            values.len(),
            self.columns.len()
        );
        self.records.push(Record {
            columns: self.columns.clone(),
            values,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}

impl fmt::Display for ResultSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.columns.is_empty() {
            return write!(f, "(empty)");
        }

        // Compute column widths
        let mut widths: Vec<usize> = self.columns.iter().map(|c| c.len()).collect();
        for record in &self.records {
            for (i, val) in record.values.iter().enumerate() {
                if i < widths.len() {
                    let w = format!("{}", val).len();
                    if w > widths[i] {
                        widths[i] = w;
                    }
                }
            }
        }

        // Header
        for (i, col) in self.columns.iter().enumerate() {
            if i > 0 {
                write!(f, " | ")?;
            }
            write!(f, "{:width$}", col, width = widths[i])?;
        }
        writeln!(f)?;

        // Separator
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                write!(f, "-+-")?;
            }
            write!(f, "{}", "-".repeat(*w))?;
        }
        writeln!(f)?;

        // Rows
        for record in &self.records {
            for (i, val) in record.values.iter().enumerate() {
                if i > 0 {
                    write!(f, " | ")?;
                }
                let s = format!("{}", val);
                if i < widths.len() {
                    write!(f, "{:width$}", s, width = widths[i])?;
                } else {
                    write!(f, "{}", s)?;
                }
            }
            writeln!(f)?;
        }

        Ok(())
    }
}