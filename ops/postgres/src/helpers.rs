//! Internal helpers for error mapping and serialization.

use ironflow_core::error::OperationError;
use serde::Serialize;
use serde_json::Value;

pub(crate) fn pg_error(e: sqlx::Error) -> OperationError {
    OperationError::External {
        origin: "postgres".to_string(),
        message: e.to_string(),
    }
}

pub(crate) fn validate_identifier(name: &str) -> Result<(), OperationError> {
    if name.is_empty() {
        return Err(OperationError::External {
            origin: "postgres".to_string(),
            message: "identifier must not be empty".to_string(),
        });
    }
    if name.contains('\0') {
        return Err(OperationError::External {
            origin: "postgres".to_string(),
            message: format!("identifier contains null byte: {name:?}"),
        });
    }
    Ok(())
}

pub(crate) fn quote_identifier(name: &str) -> Result<String, OperationError> {
    validate_identifier(name)?;
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}

pub(crate) fn to_value<T: Serialize>(v: &T) -> Result<Value, OperationError> {
    serde_json::to_value(v).map_err(|e| OperationError::External {
        origin: "postgres".to_string(),
        message: e.to_string(),
    })
}

pub(crate) fn bind_json_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match value {
        Value::Null => query.bind(None::<String>),
        Value::Bool(b) => query.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.bind(i)
            } else if let Some(f) = n.as_f64() {
                query.bind(f)
            } else {
                query.bind(n.to_string())
            }
        }
        Value::String(s) => query.bind(s.as_str()),
        _ => query.bind(value.clone()),
    }
}

pub(crate) fn row_to_json(row: &sqlx::postgres::PgRow) -> Result<Value, OperationError> {
    use serde_json::Map;
    use sqlx::{Column, Row};

    let columns = row.columns();
    let mut map = Map::with_capacity(columns.len());
    for (i, col) in columns.iter().enumerate() {
        let name = col.name().to_string();
        let val = column_to_json(row, i)?;
        map.insert(name, val);
    }
    Ok(Value::Object(map))
}

pub(crate) fn column_to_json(
    row: &sqlx::postgres::PgRow,
    idx: usize,
) -> Result<Value, OperationError> {
    use sqlx::{Column, Row, TypeInfo};

    let col = &row.columns()[idx];
    let type_name = col.type_info().name();

    macro_rules! try_get {
        ($t:ty) => {
            row.try_get::<Option<$t>, _>(idx)
                .map_err(pg_error)
                .map(|v| match v {
                    Some(val) => serde_json::to_value(val).unwrap_or(Value::Null),
                    None => Value::Null,
                })
        };
    }

    match type_name {
        "BOOL" => try_get!(bool),
        "INT2" | "SMALLINT" | "SMALLSERIAL" => try_get!(i16),
        "INT4" | "INT" | "INTEGER" | "SERIAL" => try_get!(i32),
        "INT8" | "BIGINT" | "BIGSERIAL" => try_get!(i64),
        "FLOAT4" | "REAL" => try_get!(f32),
        "FLOAT8" | "DOUBLE PRECISION" => try_get!(f64),
        "JSON" | "JSONB" => try_get!(Value),
        _ => try_get!(String),
    }
}
