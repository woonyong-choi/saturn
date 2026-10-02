//! `saturn train`: judge 학습. `/train`과 같다.
//! 설계: docs/design/judge-training.md

use std::io::Write;

use saturn_protocol::rpc::{Notification, Request};
use saturn_tui::client::EngineClient;

use crate::args::TrainArgs;
use crate::commands::{call, confirm_on_terminal};

// cost: time O(p), heap O(1), stack O(1), io p
// vars: p = 받은 진행 알림 수
// basis: estimate
/// TODO(#47): 거절과 승격 실패의 종료 코드. 학습이 끝났을 때 승격 여부를 알리는 알림도 아직 없다
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &TrainArgs) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let mut progress = std::io::stderr().lock();
    train(client, args, confirm_on_terminal, &mut out, &mut progress).await
}

// cost: time O(p), heap O(1), stack O(1), io p
// vars: p = 받은 진행 알림 수
// basis: estimate
async fn train(
    client: &mut EngineClient,
    args: &TrainArgs,
    confirm: impl FnOnce(&str) -> anyhow::Result<bool>,
    out: &mut impl Write,
    progress: &mut impl Write,
) -> anyhow::Result<()> {
    let request = Request::Train {
        reset_thresholds: args.reset_thresholds,
        from: args.from.clone(),
    };
    let mut preview = None;
    call(client, request, |notification| {
        if let Notification::TrainPreview { .. } = notification {
            preview = Some(notification);
        }
    })
    .await?;
    let Some(Notification::TrainPreview {
        candidates,
        grader,
        estimated_tokens,
        threshold_targets,
        retrain_model,
    }) = preview
    else {
        writeln!(out, "train request accepted")?;
        return Ok(());
    };
    let targets = if threshold_targets.is_empty() {
        "-".to_owned()
    } else {
        threshold_targets.join(", ")
    };
    writeln!(out, "candidates: {candidates}")?;
    writeln!(out, "grading model: {grader}")?;
    writeln!(out, "estimated tokens: {estimated_tokens}")?;
    writeln!(out, "threshold targets: {targets}")?;
    writeln!(out, "retrain model: {retrain_model}")?;

    let proceed = match confirm("run training?") {
        Ok(proceed) => proceed,
        Err(error) => {
            call(client, Request::ConfirmTrain { proceed: false }, drop).await?;
            return Err(error);
        }
    };
    call(client, Request::ConfirmTrain { proceed }, |notification| {
        if let Notification::TrainProgress {
            stage,
            labeled,
            elapsed_ms,
            tokens,
        } = notification
        {
            let _ = writeln!(
                progress,
                "{stage} · labeled {labeled} · {}s · {tokens} tokens",
                elapsed_ms / 1000
            ); // 진행 줄을 못 써도 학습은 계속한다
        }
    })
    .await?;
    let outcome = if proceed {
        "training finished"
    } else {
        "training cancelled"
    };
    writeln!(out, "{outcome}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::testing::{FakeEngine, Reply};

    use super::*;

    fn preview() -> Notification {
        Notification::TrainPreview {
            candidates: 250,
            grader: "grader-a".to_owned(),
            estimated_tokens: 12_000,
            threshold_targets: vec!["edit".to_owned()],
            retrain_model: false,
        }
    }

    fn args() -> TrainArgs {
        TrainArgs {
            reset_thresholds: false,
            from: Some("v1".to_owned()),
        }
    }

    fn progress_note() -> Notification {
        Notification::TrainProgress {
            stage: "grading".to_owned(),
            labeled: 40,
            elapsed_ms: 3_000,
            tokens: 900,
        }
    }

    #[tokio::test]
    async fn train_confirmed_sends_confirm_and_prints_progress_to_progress_stream() {
        let engine = FakeEngine::start(vec![
            Reply::with(vec![preview()]),
            Reply::with(vec![progress_note()]),
        ]);
        let mut client = engine.client().await;
        let (mut out, mut progress) = (Vec::new(), Vec::new());

        train(&mut client, &args(), |_| Ok(true), &mut out, &mut progress)
            .await
            .unwrap();

        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("candidates: 250"));
        assert!(out.ends_with("training finished\n"));
        assert_eq!(
            String::from_utf8(progress).unwrap(),
            "grading · labeled 40 · 3s · 900 tokens\n"
        );
        assert_eq!(
            engine.finish().await,
            vec![
                Request::Train {
                    reset_thresholds: false,
                    from: Some("v1".to_owned())
                },
                Request::ConfirmTrain { proceed: true }
            ]
        );
    }

    #[tokio::test]
    async fn train_declined_cancels_on_engine() {
        let engine = FakeEngine::start(vec![Reply::with(vec![preview()]), Reply::ok()]);
        let mut client = engine.client().await;
        let mut out = Vec::new();

        train(
            &mut client,
            &args(),
            |_| Ok(false),
            &mut out,
            &mut Vec::new(),
        )
        .await
        .unwrap();

        assert!(
            String::from_utf8(out)
                .unwrap()
                .ends_with("training cancelled\n")
        );
        let sent = engine.finish().await;
        assert_eq!(sent[1], Request::ConfirmTrain { proceed: false });
    }

    #[tokio::test]
    async fn train_rejected_by_engine_is_error_without_confirmation() {
        let engine = FakeEngine::start(vec![Reply::error(-32603, "50 more judgments needed")]);
        let mut client = engine.client().await;

        let error = train(
            &mut client,
            &args(),
            |_| panic!("must not ask"),
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("50 more judgments needed"));
    }

    #[tokio::test]
    async fn train_without_terminal_cancels_on_engine_and_fails() {
        let engine = FakeEngine::start(vec![Reply::with(vec![preview()]), Reply::ok()]);
        let mut client = engine.client().await;

        let result = train(
            &mut client,
            &args(),
            |_| anyhow::bail!("confirmation needs a terminal"),
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await;

        assert!(result.is_err());
        let sent = engine.finish().await;
        assert_eq!(sent[1], Request::ConfirmTrain { proceed: false });
    }
}
