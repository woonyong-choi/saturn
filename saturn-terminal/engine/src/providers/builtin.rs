//! 기본으로 등록하는 어댑터. 어댑터를 더할 때 이 파일에 등록 한 줄을 더한다.
//! 설계: docs/design/providers-and-sessions.md#어댑터-등록

pub use super::claude::{HookInputError, run_pre_tool_use};
use super::{Registry, claude, codex};

impl Registry {
    /// Saturn이 함께 내는 어댑터를 등록한 레지스트리.
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry.register_or_log(claude::adapter());
        registry.register_or_log(codex::adapter());
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_lists_claude_before_codex() {
        let registry = Registry::builtin();

        let names: Vec<&str> = registry
            .descriptors()
            .map(|descriptor| descriptor.display_name)
            .collect();

        assert_eq!(names, ["claude", "codex"]);
    }

    #[test]
    fn builtin_descriptors_carry_the_values_the_common_code_used_to_branch_on() {
        let registry = Registry::builtin();

        let facts: Vec<(&str, &str, &str, u64)> = registry
            .descriptors()
            .map(|descriptor| {
                (
                    descriptor.id.as_str(),
                    descriptor.program,
                    descriptor.instruction_doc,
                    descriptor.context.window,
                )
            })
            .collect();

        assert_eq!(
            facts,
            [
                ("claude", "claude", "CLAUDE.md", 1_000_000),
                ("codex", "codex", "AGENTS.md", 272_000)
            ]
        );
    }
}
