//! 채팅마다 TUI가 붙을 때 넘긴 작업 폴더와 환경 변수. 설계: docs/design/engine-lifecycle.md

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use saturn_protocol::ids::ChatId;

use crate::Engine;
use crate::secrets;

/// 그 채팅에 가장 나중에 붙은 TUI의 값이다. TUI가 떨어져도 채팅이 남아 있는 동안 둔다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatEnv {
    workdir: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl ChatEnv {
    pub(crate) fn new(workdir: PathBuf, env: Vec<(String, String)>) -> Self {
        Self {
            workdir,
            env: env
                .into_iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect(),
        }
    }

    pub(crate) fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// 넘겨받은 환경에 judge 키 변수가 있어도 뺀다.
    pub(crate) fn provider_env(&self) -> Vec<(OsString, OsString)> {
        secrets::scrub(self.env.iter().cloned())
    }
}

impl Engine {
    /// 붙은 적이 없는 채팅이면 `None`.
    /// TODO(#148): provider 실행 때 `LaunchSpec`의 `workdir`, `env`를 이 값으로 채운다
    pub(crate) fn chat_env(&self, chat: ChatId) -> Option<&ChatEnv> {
        self.chats.get(&chat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_env_drops_judge_key_variable_and_keeps_the_rest() {
        let env = ChatEnv::new(
            PathBuf::from("/work"),
            vec![
                ("PATH".to_owned(), "/opt/bin:/usr/bin".to_owned()),
                (secrets::JUDGE_KEY_ENV.to_owned(), "sk-secret".to_owned()),
            ],
        );

        let provider = env.provider_env();

        assert_eq!(
            provider,
            vec![(OsString::from("PATH"), OsString::from("/opt/bin:/usr/bin"))]
        );
        assert_eq!(env.workdir(), Path::new("/work"));
    }
}
