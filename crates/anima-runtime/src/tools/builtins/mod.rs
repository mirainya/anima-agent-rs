//! 内置工具集合
//!
//! 提供 6 个核心工具：bash_exec, file_read, file_write, file_edit, glob_search, grep_search。

mod bash_exec;
mod file_edit;
mod file_read;
mod file_write;
mod glob_search;
mod grep_search;

use std::path::{Component, PathBuf};
use std::sync::Arc;

use super::registry::ToolRegistry;

/// 注册所有内置工具到 ToolRegistry
pub fn register_all(registry: &mut ToolRegistry) {
    registry.register(Arc::new(bash_exec::BashExecTool));
    registry.register(Arc::new(file_read::FileReadTool));
    registry.register(Arc::new(file_write::FileWriteTool));
    registry.register(Arc::new(file_edit::FileEditTool));
    registry.register(Arc::new(glob_search::GlobSearchTool));
    registry.register(Arc::new(grep_search::GrepSearchTool));
}

/// 路径安全校验：拒绝含 ".." 组件的路径
fn validate_path_safe(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return Err(format!(
                "path traversal rejected: '..' component found in '{path}'"
            ));
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_safe_ok() {
        assert!(validate_path_safe("/tmp/foo/bar.txt").is_ok());
        assert!(validate_path_safe("src/main.rs").is_ok());
        assert!(validate_path_safe("./relative/path").is_ok());
    }

    #[test]
    fn test_validate_path_safe_rejects_traversal() {
        assert!(validate_path_safe("../etc/passwd").is_err());
        assert!(validate_path_safe("/tmp/../etc/passwd").is_err());
        assert!(validate_path_safe("foo/../../bar").is_err());
    }

    #[test]
    fn test_register_all_populates_registry() {
        let mut registry = ToolRegistry::new();
        register_all(&mut registry);
        assert_eq!(registry.len(), 6);

        let names = registry.list_names();
        assert!(names.contains(&"bash_exec"));
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"file_write"));
        assert!(names.contains(&"file_edit"));
        assert!(names.contains(&"glob_search"));
        assert!(names.contains(&"grep_search"));
    }
}
