//! 하위 명령별 실행. 결과는 stdout, 진단은 stderr.

use std::io::{BufRead, IsTerminal, Write};

use saturn_protocol::rpc::{Notification, Request};
use saturn_tui::client::{ClientError, EngineClient};

pub(crate) mod chat;
pub(crate) mod export;
pub(crate) mod judge;
pub(crate) mod prune;
pub(crate) mod train;
pub(crate) mod usage;

/// engine의 judge 키 대기 오류 번호(`-32001`, 초안)와 같다.
const JUDGE_KEY_REQUIRED: i32 = -32001;

// cost: time O(m), heap O(1), stack O(1), io m
// vars: m = 응답이 오기까지 받은 알림 수
// basis: estimate
/// 요청 하나를 보내고 응답까지 받은 알림을 `on_notification`에 넘긴다.
/// 화면이 없어 키를 묻지 않으므로 judge 키 대기 거절에는 설정 방법을 안내한다.
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn call(
    client: &mut EngineClient,
    request: Request,
    on_notification: impl FnMut(Notification),
) -> anyhow::Result<()> {
    match client.call(request, on_notification).await {
        Err(ClientError::Rejected {
            code: JUDGE_KEY_REQUIRED,
            message,
        }) => anyhow::bail!(
            "judge key required ({message}): set the SATURN_KEY environment variable or the judge.key.command setting, then run again"
        ),
        other => Ok(other?),
    }
}

/// 입력할 수 있는 터미널에서만 확인을 받는다.
///
/// # Errors
/// 표준 입력이 터미널이 아니거나 읽지 못하면 오류.
pub(crate) fn confirm_on_terminal(prompt: &str) -> anyhow::Result<bool> {
    let stdin = std::io::stdin();
    confirm_from(
        prompt,
        stdin.is_terminal(),
        stdin.lock(),
        &mut std::io::stderr(),
    )
}

/// TODO(#57): 화면이 없는 환경(파이프, CI)에서 확인 한 줄을 받는 방식. 지금은 거절한다
fn confirm_from(
    prompt: &str,
    interactive: bool,
    mut input: impl BufRead,
    prompt_out: &mut impl Write,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        interactive,
        "confirmation needs a terminal and none is available; nothing was changed"
    );
    write!(prompt_out, "{prompt} [y/N] ")?;
    prompt_out.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_from_yes_returns_true() {
        let mut prompt = Vec::new();

        let answer = confirm_from("go?", true, "y\n".as_bytes(), &mut prompt).unwrap();

        assert!(answer);
        assert_eq!(String::from_utf8(prompt).unwrap(), "go? [y/N] ");
    }

    #[test]
    fn confirm_from_empty_or_other_answer_returns_false() {
        for line in ["\n", "n\n", "maybe\n", ""] {
            let answer = confirm_from("go?", true, line.as_bytes(), &mut Vec::new()).unwrap();

            assert!(!answer, "{line:?}");
        }
    }

    #[test]
    fn confirm_from_without_terminal_is_error_and_does_not_read() {
        let error = confirm_from("go?", false, "y\n".as_bytes(), &mut Vec::new()).unwrap_err();

        assert!(error.to_string().contains("needs a terminal"));
    }
}
