// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::HarnessMode;
use kanari_core::CheckpointProductionInfo;
use std::fmt;

#[derive(Debug, Clone)]
pub struct HarnessReport {
    pub requested_txs: usize,
    pub mode: HarnessMode,
    pub block_info: CheckpointProductionInfo,
    pub duration_secs: f64,
    pub submit_secs: Option<f64>,
    pub produce_secs: Option<f64>,
    pub tps: f64,
    pub target_tps: Option<f64>,
}

impl HarnessReport {
    pub fn target_status(&self) -> &'static str {
        match self.target_tps {
            Some(target) if self.tps >= target => "pass",
            Some(_) => "fail",
            None => "not-set",
        }
    }

    pub fn render_text(&self) -> String {
        let breakdown = match (self.submit_secs, self.produce_secs) {
            (Some(submit), Some(produce)) => {
                format!("\nsubmit_secs={submit:.6}\nproduce_secs={produce:.6}")
            }
            _ => String::new(),
        };
        let target = self
            .target_tps
            .map(|target| {
                format!(
                    "\ntarget_tps={target:.2}\ntarget_status={}",
                    self.target_status()
                )
            })
            .unwrap_or_default();
        format!(
            "Kanari benchmark completed\nmode={}\nrequested_txs={}\nexecuted={}\nfailed={}\ntx_count={}\nduration_secs={:.3}{}\ntps={:.2}{}",
            self.mode.as_str(),
            self.requested_txs,
            self.block_info.executed,
            self.block_info.failed,
            self.block_info.tx_count,
            self.duration_secs,
            breakdown,
            self.tps,
            target
        )
    }

    pub fn render_json(&self) -> String {
        let timing_fields = match (self.submit_secs, self.produce_secs) {
            (Some(submit), Some(produce)) => {
                format!("\"submit_secs\":{submit:.6},\"produce_secs\":{produce:.6},")
            }
            _ => String::new(),
        };
        let target_fields = self
            .target_tps
            .map(|target| {
                format!(
                    "\"target_tps\":{target:.6},\"target_status\":\"{}\",",
                    self.target_status()
                )
            })
            .unwrap_or_default();
        format!(
            concat!(
                "{{",
                "\"requested_txs\":{},",
                "\"mode\":\"{}\",",
                "\"executed\":{},",
                "\"failed\":{},",
                "\"tx_count\":{},",
                "\"duration_secs\":{:.6},",
                "{}",
                "{}",
                "\"tps\":{:.6}",
                "}}"
            ),
            self.requested_txs,
            self.mode.as_str(),
            self.block_info.executed,
            self.block_info.failed,
            self.block_info.tx_count,
            self.duration_secs,
            timing_fields,
            target_fields,
            self.tps
        )
    }
}

impl fmt::Display for HarnessReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_renders_json_with_core_fields() {
        let report = HarnessReport {
            requested_txs: 5,
            mode: HarnessMode::Immediate,
            block_info: CheckpointProductionInfo {
                vertex_id: "vertex".to_string(),
                round: 1,
                tx_count: 5,
                executed: 5,
                failed: 0,
                events: vec![],
                checkpoint: None,
                vertex: None,
            },
            duration_secs: 0.5,
            submit_secs: None,
            produce_secs: None,
            tps: 10.0,
            target_tps: Some(5.0),
        };

        let json = report.render_json();
        assert!(json.contains("\"requested_txs\":5"));
        assert!(json.contains("\"mode\":\"immediate\""));
        assert!(json.contains("\"executed\":5"));
        assert!(json.contains("\"target_tps\":5.000000"));
        assert!(json.contains("\"target_status\":\"pass\""));
        assert!(json.contains("\"tps\":10.000000"));
    }
}
