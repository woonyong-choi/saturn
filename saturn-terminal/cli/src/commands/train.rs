//! `saturn router train`: router 학습. `/train`과 같다.
//! 설계: docs/design/router-training.md

use std::io::Write;
use std::time::Duration;

use saturn_protocol::rpc::{Notification, Request};
use saturn_tui::client::EngineClient;
use saturn_tui::i18n::{self, Lang};

use crate::args::TrainArgs;
use crate::commands::{call, confirm_on_terminal};
use crate::exit::{Exit, ExitCode};

// cost: time O(p), heap O(1), stack O(1), io p
// vars: p = 받은 진행 알림 수
// basis: estimate
/// TODO(#47): 학습이 끝났을 때 승격 여부를 알리는 알림이 아직 없어 승격 실패는 종료 코드로 알리지 못한다
///
/// # Errors
/// engine이 거절했거나, 확인에 아니라고 답했거나, 연결이 끊기면 오류.
pub(crate) async fn run(
    lang: Lang,
    client: &mut EngineClient,
    args: &TrainArgs,
) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let mut progress = std::io::stderr().lock();
    if args.yes {
        train(lang, client, args, |_| Ok(true), &mut out, &mut progress).await
    } else {
        let confirm = |prompt: &str| confirm_on_terminal(lang, prompt);
        train(lang, client, args, confirm, &mut out, &mut progress).await
    }
}

// cost: time O(p), heap O(1), stack O(1), io p
// vars: p = 받은 진행 알림 수
// basis: estimate
async fn train(
    lang: Lang,
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
    call(lang, client, request, |notification| {
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
        writeln!(out, "{}", lang.tr(i18n::CLI_TRAIN_ACCEPTED))?;
        return Ok(());
    };
    let targets = if threshold_targets.is_empty() {
        "-".to_owned()
    } else {
        threshold_targets.join(", ")
    };
    let retrain_model = lang.tr(if retrain_model { i18n::YES } else { i18n::NO });
    writeln!(out, "{}: {candidates}", lang.tr(i18n::TRAIN_CANDIDATES))?;
    writeln!(out, "{}: {grader}", lang.tr(i18n::TRAIN_GRADER))?;
    writeln!(out, "{}: {estimated_tokens}", lang.tr(i18n::TRAIN_TOKENS))?;
    writeln!(out, "{}: {targets}", lang.tr(i18n::TRAIN_TARGETS))?;
    writeln!(out, "{}: {retrain_model}", lang.tr(i18n::TRAIN_RETRAIN))?;

    let proceed = match confirm(lang.tr(i18n::CLI_TRAIN_PROMPT)) {
        Ok(proceed) => proceed,
        Err(error) => {
            call(lang, client, Request::ConfirmTrain { proceed: false }, drop).await?;
            return Err(error);
        }
    };
    call(
        lang,
        client,
        Request::ConfirmTrain { proceed },
        |notification| {
            if let Notification::TrainProgress {
                stage,
                labeled,
                elapsed_ms,
                tokens,
            } = notification
            {
                let line = lang
                    .tr(i18n::CLI_TRAIN_PROGRESS)
                    .replace("{stage}", &stage)
                    .replace("{labeled}", &labeled.to_string())
                    .replace(
                        "{elapsed}",
                        &i18n::format_elapsed(lang, Duration::from_millis(elapsed_ms)),
                    )
                    .replace("{tokens}", &tokens.to_string());
                let _ = writeln!(progress, "{line}"); // 진행 줄을 못 써도 학습은 계속한다
            }
        },
    )
    .await?;
    if !proceed {
        return Err(Exit::error(
            ExitCode::Failure,
            lang.tr(i18n::CLI_TRAIN_CANCELLED),
        ));
    }
    writeln!(out, "{}", lang.tr(i18n::CLI_TRAIN_FINISHED))?;
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
            yes: false,
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

        train(
            Lang::En,
            &mut client,
            &args(),
            |_| Ok(true),
            &mut out,
            &mut progress,
        )
        .await
        .unwrap();

        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("Candidates: 250"));
        assert!(out.contains("Retrain model: No"));
        assert!(out.ends_with("Training finished\n"));
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

        let error = train(
            Lang::En,
            &mut client,
            &args(),
            |_| Ok(false),
            &mut out,
            &mut Vec::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "Training cancelled");
        assert_eq!(crate::exit::of(&error), ExitCode::Failure);
        let sent = engine.finish().await;
        assert_eq!(sent[1], Request::ConfirmTrain { proceed: false });
    }

    #[tokio::test]
    async fn train_rejected_by_engine_is_error_without_confirmation() {
        let engine = FakeEngine::start(vec![Reply::error(-32603, "50 more judgments needed")]);
        let mut client = engine.client().await;

        let error = train(
            Lang::En,
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
            Lang::En,
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
