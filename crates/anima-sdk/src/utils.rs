use anima_types::{AnimaError, ApiErrorKind, ApiResponse, Result};
use serde_json::{Map, Value};

pub fn handle_response(response: ApiResponse) -> Result<Value> {
    if response.success {
        Ok(response.data.unwrap_or(Value::Null))
    } else {
        match response.error {
            Some(ApiErrorKind::BadRequest) => Err(AnimaError::BadRequest { response }),
            Some(ApiErrorKind::NotFound) => Err(AnimaError::NotFound { response }),
            Some(ApiErrorKind::ServerError) => Err(AnimaError::ServerError { response }),
            _ => Err(AnimaError::Unknown { response }),
        }
    }
}

pub fn validate_required(params: &Map<String, Value>, required_keys: &[&str]) -> Result<()> {
    let missing: Vec<&str> = required_keys
        .iter()
        .copied()
        .filter(|key| params.get(*key).is_none() || params.get(*key) == Some(&Value::Null))
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(AnimaError::MissingRequired(missing.join(", ")))
    }
}
