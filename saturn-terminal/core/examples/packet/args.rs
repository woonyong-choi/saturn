//! `packet` 인자. 입력은 표준 입력의 기록 JSONL이거나 `--scenarios` 파일이다.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use saturn_core::sessions::ranking::DEFAULT_RRF_K;

/// 개발용 패킷 생성. 실험 수집기가 `SATURN_PACKET_CMD`로 불러 쓴다.
#[derive(Debug, Parser)]
#[command(name = "packet")]
pub(crate) struct PacketArgs {
    /// `saturn`은 Saturn 기록 원문으로, `provider`는 `--summary-file`의 요약을 경쟁 구역 첫 항목으로 채운다.
    #[arg(long, value_enum, default_value_t = PacketMode::Saturn)]
    pub(crate) mode: PacketMode,
    /// 패킷 상한 `P_max`(토큰). 발동 기준 `T`는 이 값의 10배로 둔다.
    #[arg(long, visible_alias = "budget", value_name = "TOKENS")]
    pub(crate) budget_tokens: u64,
    /// 경쟁 구역의 순서 규칙. 없으면 `--judgments`가 있을 때 `router-only`, 없을 때 `rrf-only`다.
    #[arg(long, value_enum)]
    pub(crate) condition: Option<PacketCondition>,
    /// `rrf-router`에서 router 판단을 쓰는 RRF 상위 후보 수.
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub(crate) top_n: usize,
    /// RRF 합치기 상수.
    #[arg(long, value_name = "K", default_value_t = DEFAULT_RRF_K)]
    pub(crate) k: u32,
    /// router 판단 JSON(`compact`, `constraints`). 없으면 RRF 순서로 채운다.
    #[arg(long, value_name = "FILE")]
    pub(crate) judgments: Option<PathBuf>,
    /// 시나리오 JSONL. 있으면 표준 입력 대신 이 파일의 `--scenario-id` 시나리오를 읽는다.
    #[arg(long, value_name = "FILE", requires = "scenario_id")]
    pub(crate) scenarios: Option<PathBuf>,
    /// `--scenarios`에서 고를 시나리오.
    #[arg(long, value_name = "ID", requires = "scenarios")]
    pub(crate) scenario_id: Option<String>,
    /// provider 압축 요약 글 파일.
    #[arg(long, value_name = "FILE", requires = "summary_record")]
    pub(crate) summary_file: Option<PathBuf>,
    /// 요약 시점의 마지막 기록 번호. 이 번호 뒤의 기록에서만 경쟁 구역을 고른다.
    #[arg(long, value_name = "RECORD_NO", requires = "summary_file")]
    pub(crate) summary_record: Option<u64>,
    /// 출력 형식. 없으면 `--scenarios`가 있을 때 `json`, 없을 때 `text`다.
    #[arg(long, value_enum)]
    pub(crate) format: Option<PacketFormat>,
}

/// 패킷 정리 모드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketMode {
    /// Saturn 기록 원문에서 고른다.
    Saturn,
    /// provider 압축 요약을 첫 항목으로 쓴다.
    Provider,
}

/// 경쟁 구역 순서 조건.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketCondition {
    /// router 판단을 쓰지 않고 RRF 순서로 채운다.
    RrfOnly,
    /// RRF 상위 `--top-n`개의 판단만 쓴다.
    RrfRouter,
    /// 후보 전체의 판단을 쓴다.
    RouterOnly,
}

/// 패킷 출력 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketFormat {
    /// 패킷 글만.
    Text,
    /// 패킷 글과 넣은 항목, RRF 순서를 담은 JSON.
    Json,
}
