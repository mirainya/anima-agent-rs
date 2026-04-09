/// Get the prefix of a context key (part before first ':')
pub fn key_prefix(key: &str) -> Option<&str> {
    key.split(':')
        .next()
        .filter(|p| !p.is_empty() && key.contains(':'))
}

/// Split a context key into parts by ':'
pub fn key_parts(key: &str) -> Vec<&str> {
    key.split(':').collect()
}

/// Match a key against a pattern supporting '*' (any chars) and '?' (single char)
pub fn matches_pattern(key: &str, pattern: &str) -> bool {
    match_recursive(key.as_bytes(), pattern.as_bytes())
}

fn match_recursive(key: &[u8], pattern: &[u8]) -> bool {
    match (pattern.first(), key.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            // '*' matches zero or more characters
            match_recursive(key, &pattern[1..])
                || (!key.is_empty() && match_recursive(&key[1..], pattern))
        }
        (Some(b'?'), Some(_)) => match_recursive(&key[1..], &pattern[1..]),
        (Some(p), Some(k)) if p == k => match_recursive(&key[1..], &pattern[1..]),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContextType {
    Session,
    Task,
    Agent,
    Cache,
    Unknown,
}

/// Determine the context type from a key's prefix
pub fn context_type(key: &str) -> ContextType {
    match key_prefix(key) {
        Some("session") => ContextType::Session,
        Some("task") => ContextType::Task,
        Some("agent") => ContextType::Agent,
        Some("cache") => ContextType::Cache,
        _ => ContextType::Unknown,
    }
}
