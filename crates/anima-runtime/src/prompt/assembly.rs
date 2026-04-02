//! 段落化 System Prompt 组装器
//!
//! 收集多个 `PromptSection`，按 `order` 排序后以 `"\n\n"` 拼接为最终文本。

use super::types::{PromptSection, SystemPrompt};

/// System Prompt 组装器
#[derive(Debug, Default)]
pub struct PromptAssembler {
    sections: Vec<PromptSection>,
}

impl PromptAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加一个段落
    pub fn add_section(&mut self, section: PromptSection) -> &mut Self {
        self.sections.push(section);
        self
    }

    /// 快捷方法：直接传入 id、内容、排序权重
    pub fn add_text(&mut self, id: &str, content: &str, order: i32) -> &mut Self {
        self.sections.push(PromptSection {
            id: id.to_string(),
            content: content.to_string(),
            order,
        });
        self
    }

    /// 按 order 排序，拼接所有段落
    pub fn build(&self) -> SystemPrompt {
        let mut sorted: Vec<&PromptSection> = self.sections.iter().collect();
        sorted.sort_by_key(|s| s.order);

        let section_ids: Vec<String> = sorted.iter().map(|s| s.id.clone()).collect();
        let text = sorted
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        SystemPrompt { text, section_ids }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_assembler() {
        let asm = PromptAssembler::new();
        let result = asm.build();
        assert!(result.text.is_empty());
        assert!(result.section_ids.is_empty());
    }

    #[test]
    fn test_single_section() {
        let mut asm = PromptAssembler::new();
        asm.add_section(PromptSection {
            id: "intro".into(),
            content: "Hello world".into(),
            order: 0,
        });
        let result = asm.build();
        assert_eq!(result.text, "Hello world");
        assert_eq!(result.section_ids, vec!["intro"]);
    }

    #[test]
    fn test_ordering() {
        let mut asm = PromptAssembler::new();
        asm.add_section(PromptSection {
            id: "last".into(),
            content: "C".into(),
            order: 200,
        });
        asm.add_section(PromptSection {
            id: "first".into(),
            content: "A".into(),
            order: 0,
        });
        asm.add_section(PromptSection {
            id: "middle".into(),
            content: "B".into(),
            order: 100,
        });

        let result = asm.build();
        assert_eq!(result.text, "A\n\nB\n\nC");
        assert_eq!(result.section_ids, vec!["first", "middle", "last"]);
    }

    #[test]
    fn test_add_text_convenience() {
        let mut asm = PromptAssembler::new();
        asm.add_text("greeting", "Hi there", 10);
        let result = asm.build();
        assert_eq!(result.text, "Hi there");
        assert_eq!(result.section_ids, vec!["greeting"]);
    }
}
