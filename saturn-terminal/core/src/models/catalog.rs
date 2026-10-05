use serde::{Deserialize, Serialize};

/// 근거가 어디서 왔는지. 품질을 확정할 수 있는 종류는 `Benchmark`와 `OwnExperiment`뿐이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// 외부 벤치마크 점수.
    Benchmark,
    /// 공개 사용 평가와 여론. 발견과 재검증 우선순위의 참고일 뿐이다.
    PublicOpinion,
    /// provider가 공식으로 밝힌 지원 기능, 한도, 가격.
    OfficialSpec,
    /// Saturn 자체 실험 결과.
    OwnExperiment,
}

/// 근거 한 건. 출처와 수집일은 항상 적는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    /// 벤치마크 이름이나 글 제목.
    pub name: String,
    pub source_url: String,
    /// 수집한 날 `YYYY-MM-DD`.
    pub collected: String,
    /// 벤치마크나 자료의 버전. 없으면 품질 확정에 쓰지 않는다.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    /// 표본 크기나 구성.
    #[serde(default)]
    pub sample: Option<String>,
    /// 실행 환경(도구, 추론 깊이 등).
    #[serde(default)]
    pub environment: Option<String>,
    /// 여론의 반례.
    #[serde(default)]
    pub counterexample: Option<String>,
    /// 이 근거를 잰 모델의 revision. 항목의 revision과 같을 때만 그 모델의 근거다.
    #[serde(default)]
    pub model_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Price {
    /// `usd_per_million_tokens`처럼 단위를 글로 적는다.
    pub unit: String,
    pub input: f64,
    pub output: f64,
}

/// 모델 하나. provider가 알려 주는 모델 이름(`model_id`나 `alias`)으로 찾는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// provider id 글자(`claude`, `codex`).
    pub provider: String,
    pub model_id: String,
    /// provider 목록에 보이는 별칭. 별칭이 가리키는 모델은 바뀔 수 있다.
    #[serde(default)]
    pub alias: Option<String>,
    /// 이 항목이 설명하는 고정 revision. 모르면 비운다.
    #[serde(default)]
    pub revision: Option<String>,
    /// 지원하는 추론 깊이.
    #[serde(default)]
    pub efforts: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub price: Option<Price>,
    #[serde(default)]
    pub uncertainty: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

/// 검토를 거쳐 버전으로 배포하는 목록. 실행 중에는 바뀌지 않는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub version: String,
    /// 자료를 모은 날 `YYYY-MM-DD`.
    pub collected: String,
    /// 이 날 뒤에는 모든 모델이 미검증이다.
    pub expires: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog version should not be empty")]
    EmptyVersion,
    #[error("`{0}` should be a date like 2026-10-06")]
    BadDate(String),
    #[error("catalog entry `{0}` should not appear twice")]
    DuplicateEntry(String),
    #[error("evidence `{0}` should have a source url and collection date")]
    MissingSource(String),
}

/// 품질을 확정하지 못한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverified {
    NotInCatalog,
    CatalogExpired,
    /// 날짜·버전·점수가 모두 있는 벤치마크나 자체 실험이 없다.
    NoDatedEvidence,
    /// 지금 쓰는 모델의 revision을 알 수 없다.
    RevisionUnknown,
    /// 지금 쓰는 모델의 revision이 근거를 잰 revision과 다르다. 별칭이 새 모델로 바뀐 경우다.
    RevisionChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Verified,
    Unverified(Unverified),
}

/// `YYYY-MM-DD` 형식 검사. 글자 순서가 날짜 순서와 같다.
fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

impl Evidence {
    /// 품질 확정에 쓸 수 있는 근거인지. 벤치마크나 자체 실험이고 날짜, 버전, 점수가 모두 있어야 한다.
    fn confirms_quality(&self) -> bool {
        matches!(
            self.kind,
            EvidenceKind::Benchmark | EvidenceKind::OwnExperiment
        ) && is_date(&self.collected)
            && self.score.is_some()
            && self.version.as_deref().is_some_and(|text| !text.is_empty())
    }
}

impl ModelCatalog {
    /// 배포 전 검사. 날짜 형식, 중복, 출처 누락을 거른다.
    ///
    /// # Errors
    /// 처음 만난 문제.
    // cost: time O(e·v), heap O(e), stack O(1)
    // vars: e = 항목 수, v = 항목당 근거 수
    // basis: estimate
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.version.is_empty() {
            return Err(CatalogError::EmptyVersion);
        }
        for date in [&self.collected, &self.expires] {
            if !is_date(date) {
                return Err(CatalogError::BadDate(date.clone()));
            }
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let key = entry.key();
            if self.entries[..index].iter().any(|other| other.key() == key) {
                return Err(CatalogError::DuplicateEntry(key));
            }
            for evidence in &entry.evidence {
                if evidence.source_url.is_empty() || !is_date(&evidence.collected) {
                    return Err(CatalogError::MissingSource(evidence.name.clone()));
                }
            }
        }
        Ok(())
    }

    /// provider와 모델 이름(id나 별칭)으로 찾는다.
    #[must_use]
    pub fn find(&self, provider: &str, model: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| {
            entry.provider == provider
                && (entry.model_id == model || entry.alias.as_deref() == Some(model))
        })
    }

    /// 지금 쓰는 모델의 품질을 확정할 수 있는지. 확정하지 못하면 이유를 돌려준다. `today`는 `YYYY-MM-DD`.
    ///
    /// 여론과 공식 사양은 품질을 확정하지 않는다. 근거를 잰 revision과 지금 revision이 같을 때만 그 근거를 쓴다.
    // cost: time O(e + v), heap O(1), stack O(1)
    // vars: e = 항목 수, v = 항목의 근거 수
    // basis: estimate
    #[must_use]
    pub fn quality(
        &self,
        provider: &str,
        model: &str,
        current_revision: Option<&str>,
        today: &str,
    ) -> Quality {
        let Some(entry) = self.find(provider, model) else {
            return Quality::Unverified(Unverified::NotInCatalog);
        };
        if today > self.expires.as_str() {
            return Quality::Unverified(Unverified::CatalogExpired);
        }
        let Some(revision) = entry.revision.as_deref() else {
            return Quality::Unverified(Unverified::RevisionUnknown);
        };
        let measured = |evidence: &&Evidence| {
            evidence.confirms_quality() && evidence.model_revision.as_deref() == Some(revision)
        };
        if !entry.evidence.iter().any(|evidence| measured(&evidence)) {
            return Quality::Unverified(Unverified::NoDatedEvidence);
        }
        match current_revision {
            None => Quality::Unverified(Unverified::RevisionUnknown),
            Some(current) if current != revision => {
                Quality::Unverified(Unverified::RevisionChanged)
            }
            Some(_) => Quality::Verified,
        }
    }
}

impl CatalogEntry {
    fn key(&self) -> String {
        format!("{}/{}", self.provider, self.model_id)
    }
}
