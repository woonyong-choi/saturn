//! 패킷 항목 앞에 적는 기록 번호와 시각, session 제목.
//! 설계: docs/design/context-management.md#패킷-구성

use saturn_protocol::ids::{LedgerSeq, SessionId};

const MS_PER_MINUTE: i64 = 60_000;
const MINUTES_PER_DAY: i64 = 1_440;

/// 항목이 일어난 session과 기록 시각. 기록 저장소의 행은 모두 시각이 있고, 시각이 없는 입력만 `None`이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub session: SessionId,
    /// unix 밀리초, UTC.
    pub at_ms: Option<i64>,
}

// cost: time O(1), heap O(1), stack O(1), alloc 1
// basis: estimate
/// `#41 2026-09-12T10:00Z`. 분 아래는 버리고, 시각이 없으면 `#41`이다.
pub(super) fn label(seq: LedgerSeq, at_ms: Option<i64>) -> String {
    let Some(at_ms) = at_ms else {
        return format!("#{}", seq.0);
    };
    let minutes = at_ms.div_euclid(MS_PER_MINUTE);
    let (year, month, day) = civil_from_days(minutes.div_euclid(MINUTES_PER_DAY));
    let in_day = minutes.rem_euclid(MINUTES_PER_DAY);
    format!(
        "#{} {year:04}-{month:02}-{day:02}T{:02}:{:02}Z",
        seq.0,
        in_day / 60,
        in_day % 60
    )
}

/// `### Session 2`.
pub(super) fn session_title(session: SessionId) -> String {
    format!("### Session {}", session.0)
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 1970-01-01부터 센 날수를 (년, 월, 일)로 바꾼다. Howard Hinnant의 civil_from_days.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_formats_seq_and_utc_minute() {
        // (사례, 기록 번호, 시각(ms), 예상 문자열)
        let cases = [
            (
                "utc minute",
                41,
                Some(1_789_207_200_000),
                "#41 2026-09-12T10:00Z",
            ),
            (
                "drops seconds on a leap day",
                1,
                Some(1_709_164_800_000 + 59_999),
                "#1 2024-02-29T00:00Z",
            ),
            (
                "last minute of february",
                2,
                Some(1_772_323_199_000),
                "#2 2026-02-28T23:59Z",
            ),
            ("epoch", 0, Some(0), "#0 1970-01-01T00:00Z"),
            ("before epoch", 0, Some(-60_000), "#0 1969-12-31T23:59Z"),
            ("without time is seq only", 7, None, "#7"),
        ];

        for (name, seq, time, expected) in cases {
            assert_eq!(label(LedgerSeq(seq), time), expected, "{name}");
        }
    }

    #[test]
    fn session_title_names_session_number() {
        assert_eq!(session_title(SessionId(3)), "### Session 3");
    }
}
