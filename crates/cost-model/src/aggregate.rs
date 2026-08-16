use crate::{CalibrationReport, CombinedReport, CostModelError, GpuCalibrationSample};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FinalAggregateReport {
    pub schema_version: String,
    pub fixed_envelope_work_v1: Option<f64>,
    pub median_core_alpha: Option<f64>,
    pub max_observed_core_alpha: Option<f64>,
    pub estimated_312m_seconds_typical: Option<f64>,
    pub estimated_312m_seconds_worst: Option<f64>,
    pub estimated_5b_seconds_typical: Option<f64>,
    pub estimated_5b_seconds_worst: Option<f64>,
    pub sixteen_item_latency_seconds: Option<f64>,
    pub sixteen_item_total_gpu_work_seconds: Option<f64>,
    pub calibration: Option<CalibrationReport>,
}

pub fn aggregate(
    combined: &CombinedReport,
    calibration: Option<&CalibrationReport>,
) -> Result<FinalAggregateReport, CostModelError> {
    let mut alphas = combined
        .workloads
        .iter()
        .filter_map(|workload| workload.core_alpha)
        .collect::<Vec<_>>();
    if alphas.is_empty() {
        return Err(CostModelError::Correctness(
            "no complete core alpha observations".into(),
        ));
    }
    alphas.sort_by(f64::total_cmp);
    let median = alphas[alphas.len() / 2];
    let max = alphas.iter().copied().fold(0.0, f64::max);
    let fixed_envelope_work = combined
        .workloads
        .iter()
        .filter_map(|workload| {
            Some(
                workload.proof_work_v1? as f64
                    - workload.core_alpha? * workload.pvm_core_instructions as f64,
            )
        })
        .map(|value| value.max(0.0))
        .collect::<Vec<_>>();
    let fixed_envelope_work = if fixed_envelope_work.is_empty() {
        None
    } else {
        Some(fixed_envelope_work.iter().sum::<f64>() / fixed_envelope_work.len() as f64)
    };
    let estimate = |gas: f64, alpha: f64| {
        calibration.map(|model| {
            model.t0_seconds
                + model.k_seconds_per_work * (gas * alpha + fixed_envelope_work.unwrap_or_default())
        })
    };
    let typical_gas = 312_000_000.0;
    let worst_gas = 5_000_000_000.0;
    let sixteen = 16.0 * typical_gas;
    Ok(FinalAggregateReport {
        schema_version: crate::schema::SCHEMA_VERSION.to_string(),
        fixed_envelope_work_v1: fixed_envelope_work,
        median_core_alpha: Some(median),
        max_observed_core_alpha: Some(max),
        estimated_312m_seconds_typical: estimate(typical_gas, median),
        estimated_312m_seconds_worst: estimate(typical_gas, max),
        estimated_5b_seconds_typical: estimate(worst_gas, median),
        estimated_5b_seconds_worst: estimate(worst_gas, max),
        sixteen_item_latency_seconds: estimate(typical_gas, max),
        sixteen_item_total_gpu_work_seconds: estimate(sixteen, max),
        calibration: calibration.cloned(),
    })
}

pub fn fit(samples: &[GpuCalibrationSample]) -> Option<CalibrationReport> {
    if samples.len() < 2 {
        return None;
    }
    let n = samples.len() as f64;
    let mean_x = samples.iter().map(|s| s.proof_work as f64).sum::<f64>() / n;
    let mean_y = samples.iter().map(|s| s.prove_ns as f64 / 1e9).sum::<f64>() / n;
    let denominator = samples
        .iter()
        .map(|s| (s.proof_work as f64 - mean_x).powi(2))
        .sum::<f64>();
    if denominator == 0.0 {
        return None;
    }
    let k = samples
        .iter()
        .map(|s| (s.proof_work as f64 - mean_x) * (s.prove_ns as f64 / 1e9 - mean_y))
        .sum::<f64>()
        / denominator;
    let t0 = mean_y - k * mean_x;
    let predicted = samples
        .iter()
        .map(|s| t0 + k * s.proof_work as f64)
        .collect::<Vec<_>>();
    let ss_tot = samples
        .iter()
        .map(|s| (s.prove_ns as f64 / 1e9 - mean_y).powi(2))
        .sum::<f64>();
    let ss_res = samples
        .iter()
        .enumerate()
        .map(|(i, s)| (s.prove_ns as f64 / 1e9 - predicted[i]).powi(2))
        .sum::<f64>();
    let r_squared = if ss_tot == 0.0 {
        1.0
    } else {
        1.0 - ss_res / ss_tot
    };
    let max_relative_error = samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (predicted[i] - s.prove_ns as f64 / 1e9).abs()
                / (s.prove_ns as f64 / 1e9).max(f64::MIN_POSITIVE)
        })
        .fold(0.0, f64::max);
    Some(CalibrationReport {
        t0_seconds: t0,
        k_seconds_per_work: k,
        throughput_work_per_second: (1.0 / k).max(0.0),
        r_squared,
        max_relative_error,
        calibration_status: if r_squared >= 0.95 {
            "linear".into()
        } else {
            "nonlinear".into()
        },
    })
}
