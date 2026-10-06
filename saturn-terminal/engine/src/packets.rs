//! 전달 패킷 근거: provider를 부르기 직전에 보낼 패킷 한 시도를 기록 저장소에 쓰고, 결과를 알면 확정한다.
//! 받은 응답(원시 기록)과 보낸 패킷은 다른 기록이다. 이 기록은 보낸 쪽만 맡고 본문은 해시로만 남긴다.
//! 설계: docs/design/records.md#전달-패킷-근거

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{ChatId, InputId, Provider, SessionId, SettingsRevision};

use crate::Engine;
use crate::handoff::PacketEvidence;
use crate::store::{NewPacket, PacketId, PacketKind, PacketState, sha256_hex};

/// 실험용 본문 캡처를 켜는 환경 변수. 값이 `1`일 때만 켠다. 설정 파일의 키가 아니라 폴더 설정이 켤 수 없다.
pub(crate) const CAPTURE_ENV: &str = "SATURN_PACKET_CAPTURE";

/// 캡처 파일을 두는 홈 아래 폴더 이름.
const CAPTURE_DIR: &str = "packet-capture";

/// 캡처 폴더. 캡처 변수가 `1`이고 `SATURN_HOME`이 따로 지정한 홈이 이 engine의 홈과 같으며 그 홈이 기본 홈
/// (`<HOME>/.saturn`)이 아닐 때만 `Some`이다. 기본 홈과 `--home` 단독 실행에서는 켜지지 않는다.
pub(crate) fn capture_dir(
    home: &Path,
    capture: Option<OsString>,
    saturn_home: Option<OsString>,
    user_home: Option<OsString>,
) -> Option<PathBuf> {
    if capture.as_deref() != Some("1".as_ref()) {
        return None;
    }
    let isolated = saturn_protocol::home::resolve(saturn_home.filter(|v| !v.is_empty()), None)?;
    let default = saturn_protocol::home::resolve(None, user_home);
    (isolated == home && default.as_deref() != Some(home)).then(|| home.join(CAPTURE_DIR))
}

/// 캡처 폴더를 소유자 전용(0700)으로 준비한다. 이미 있던 폴더가 심볼릭 링크이거나 폴더가 아니면 따라가지 않고 거절하고,
/// 있던 폴더의 권한은 0700으로 바로잡는다.
fn private_capture_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "capture path exists and is not a plain directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        Err(error) => return Err(error),
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// 보내기 직전의 본문과 식별 값을 파일 하나(소유자만 읽고 쓰기)로 남긴다. engine이 내보낸 값이며 provider가 받았다는 증거가 아니다.
fn write_capture(
    dir: &Path,
    packet: PacketId,
    target: &PacketTarget,
    attempt: u32,
    body: &str,
) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    private_capture_dir(dir)?;
    let captured_at_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_micros());
    let record = serde_json::json!({
        "packet_id": packet.0,
        "body_hash": sha256_hex(body.as_bytes()),
        "body_bytes": body.len(),
        "chat": target.chat.0,
        "session": target.session.0,
        "input": target.input.map(|input| input.0),
        "provider": format!("{:?}", target.provider),
        "kind": format!("{:?}", target.kind),
        "attempt": attempt,
        "captured_at_unix_us": captured_at_us.to_string(),
        "evidence": "engine outbound before the provider call; not proof of provider receipt",
        "body": body,
    });
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join(format!("{}.json", packet.0)))?;
    file.write_all(record.to_string().as_bytes())
}

/// 패킷을 받는 session과 보내게 한 입력.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PacketTarget {
    pub(crate) chat: ChatId,
    pub(crate) kind: PacketKind,
    pub(crate) session: SessionId,
    pub(crate) input: Option<InputId>,
    pub(crate) provider: Provider,
    pub(crate) settings: SettingsRevision,
}

/// 결과로 시도의 상태를 정한다. 보내지 않음이 확정인 오류만 `NotSent`이고, 연결이 끊긴 경우를 포함해 나머지 오류는
/// 보냈는지 모른다.
pub(crate) fn state_of<T>(result: &Result<T, ProviderError>) -> PacketState {
    match result {
        Ok(_) => PacketState::Sent,
        Err(ProviderError::NotSent { .. } | ProviderError::ContextExceeded { .. }) => {
            PacketState::NotSent
        }
        Err(_) => PacketState::Unknown,
    }
}

impl Engine {
    /// 보낼 패킷 시도를 기록한다. 기록하지 못해도 전송은 막지 않고 로그만 남기며 `None`을 돌려준다.
    pub(crate) async fn record_packet_attempt(
        &self,
        target: PacketTarget,
        body: &str,
        evidence: &PacketEvidence,
    ) -> Option<PacketId> {
        let policy = self
            .policy_digest_at(target.settings)
            .await
            .unwrap_or_else(|_| "unavailable".to_owned());
        let recorded = async {
            let constraint_revision = self.store.constraint_revision(target.chat).await?;
            self.store
                .record_packet(&NewPacket {
                    chat: target.chat,
                    kind: target.kind,
                    attempt: evidence.attempt,
                    reduced_from: evidence.reduced_from,
                    session: target.session,
                    input: target.input,
                    provider: target.provider,
                    settings: target.settings.0,
                    chat_revision: evidence.up_to.0,
                    constraint_revision,
                    policy,
                    body: body.to_owned(),
                    estimated_tokens: evidence.tokens,
                    items: evidence.rows(),
                    selection: Some(evidence.selection()),
                })
                .await
        }
        .await;
        match recorded {
            Ok(id) => {
                if let Some(dir) = &self.packet_capture {
                    let written = write_capture(dir, id, &target, evidence.attempt, body);
                    // 본문은 로그에 남기지 않는다. 캡처가 빠진 시도는 수집기가 시도 번호로 찾아 거절한다.
                    if let Err(error) = written {
                        tracing::warn!(%error, packet = id.0, "failed to capture a handoff packet");
                    }
                }
                Some(id)
            }
            Err(error) => {
                self.warn_failure("failed to record a handoff packet", Err::<(), _>(error));
                None
            }
        }
    }

    /// 결과를 안 시도의 상태를 확정한다. 기록하지 못하면 로그만 남긴다.
    pub(crate) async fn settle_packet<T>(
        &self,
        packet: Option<PacketId>,
        result: &Result<T, ProviderError>,
        provider_session: Option<&str>,
    ) {
        let Some(packet) = packet else {
            return;
        };
        let settled = self
            .store
            .settle_packet(packet, state_of(result), provider_session)
            .await;
        self.warn_failure("failed to settle a handoff packet", settled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn capture_needs_the_flag_and_an_explicitly_isolated_home_that_is_not_the_default() {
        let home = Path::new("/exp/home");
        // (사례, 캡처 변수, SATURN_HOME, 사용자 홈, 켜짐)
        let cases = [
            (
                "flag and isolated home",
                os("1"),
                os("/exp/home"),
                os("/Users/me"),
                true,
            ),
            ("flag unset", None, os("/exp/home"), os("/Users/me"), false),
            (
                "flag is not 1",
                os("true"),
                os("/exp/home"),
                os("/Users/me"),
                false,
            ),
            (
                "home chosen only by --home",
                os("1"),
                None,
                os("/Users/me"),
                false,
            ),
            (
                "saturn home differs from engine home",
                os("1"),
                os("/other"),
                os("/Users/me"),
                false,
            ),
        ];
        for (name, flag, saturn_home, user_home, expected) in cases {
            let dir = capture_dir(home, flag, saturn_home, user_home);
            assert_eq!(dir.is_some(), expected, "{name}");
        }
        let default = Path::new("/exp/.saturn");
        assert_eq!(
            capture_dir(default, os("1"), os("/exp/.saturn"), os("/exp")),
            None,
            "the default home is never captured"
        );
    }

    #[test]
    fn capture_dir_is_private_even_when_it_exists_and_never_follows_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_capture_dir(&existing).unwrap();
        assert_eq!(mode(&existing), 0o700);

        let victim = root.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("packet-capture");
        symlink(&victim, &link).unwrap();
        let target = PacketTarget {
            chat: ChatId(1),
            kind: PacketKind::Switch,
            session: SessionId(1),
            input: None,
            provider: crate::providers::test_support::CLAUDE,
            settings: SettingsRevision(1),
        };
        assert!(write_capture(&link, PacketId(1), &target, 1, "body").is_err());
        assert_eq!(mode(&victim), 0o755, "the link target keeps its mode");
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
    }

    #[test]
    fn only_a_confirmed_refusal_is_not_sent_and_every_other_failure_is_unknown() {
        let cases: [(Result<(), ProviderError>, PacketState); 5] = [
            (Ok(()), PacketState::Sent),
            (
                Err(ProviderError::NotSent {
                    reason: "write failed".to_owned(),
                }),
                PacketState::NotSent,
            ),
            (
                Err(ProviderError::ContextExceeded { limit_tokens: None }),
                PacketState::NotSent,
            ),
            (Err(ProviderError::Unknown), PacketState::Unknown),
            (Err(ProviderError::ConnectionLost), PacketState::Unknown),
        ];

        for (result, expected) in cases {
            assert_eq!(state_of(&result), expected, "{result:?}");
        }
    }
}
