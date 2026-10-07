//! 하위 명령별 실행. 결과는 stdout, 진단은 stderr.

use saturn_protocol::envelope::ErrorKind;
use saturn_protocol::rpc::{Notification, QueryResult, Request};
use saturn_tui::client::{ClientError, EngineClient};
use saturn_tui::i18n::{self, Lang};

use crate::exit::{Exit, ExitCode};

pub(crate) mod chat;
pub(crate) mod child;
pub(crate) mod evidence;
pub(crate) mod export;
pub(crate) mod prune;
pub(crate) mod resume;
pub(crate) mod usage;

// cost: time O(m), heap O(1), stack O(1), io m
// vars: m = 응답이 오기까지 받은 알림 수
// basis: estimate
/// 요청 하나를 보내고 응답까지 받은 알림을 `on_notification`에 넘기고, 조회 요청이면 응답의 결과를 돌려준다.
/// 화면이 없어 키를 묻지 않으므로 router 키 대기 거절에는 설정 방법을 안내한다.
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn call(
    lang: Lang,
    client: &mut EngineClient,
    request: Request,
    on_notification: impl FnMut(Notification),
) -> anyhow::Result<Option<QueryResult>> {
    match client.call(request, on_notification).await {
        Err(ClientError::Rejected {
            kind: Some(ErrorKind::RouterKey),
            message,
            ..
        }) => Err(Exit::error(
            ExitCode::RouterKey,
            lang.tr(i18n::CLI_ROUTER_KEY_REQUIRED)
                .replace("{message}", &message),
        )),
        other => Ok(other?),
    }
}
