use std::env;

use serde::Deserialize;
use whim_runtime::engine::EngineConfiguration;

use crate::config::Error;

const OPTIMIZATIONS: &str = "WHIM_OPTIMIZATIONS";
const CALL_DEPTH: &str = "WHIM_CALL_DEPTH";
const CYCLE_THRESHOLD: &str = "WHIM_CYCLE_THRESHOLD";
const FULL_TRACE: &str = "WHIM_FULL_TRACE";

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OptimizationMode {
    #[default]
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RuntimeSettings {
    optimizations: OptimizationMode,
    #[serde(rename = "call-depth")]
    call_depth: Option<usize>,
    #[serde(rename = "cycle-threshold")]
    cycle_threshold: Option<usize>,
    #[serde(rename = "full-trace")]
    full_trace: bool,
}

impl RuntimeSettings {
    pub(crate) fn engine_configuration(self) -> EngineConfiguration {
        let mut configuration = EngineConfiguration {
            cycle_threshold: self.cycle_threshold,
            optimize: matches!(self.optimizations, OptimizationMode::On),
            full_trace: self.full_trace,
            ..EngineConfiguration::default()
        };
        if let Some(call_depth) = self.call_depth {
            configuration.call_depth_limit = call_depth;
        }

        configuration
    }
}

pub(super) fn apply_environment(
    mut configuration: EngineConfiguration,
) -> Result<EngineConfiguration, Error> {
    if let Some(value) = read_environment(OPTIMIZATIONS)? {
        configuration.optimize = match value.as_str() {
            "on" => true,
            "off" => false,
            _ => {
                return Err(Error::InvalidOptimizationMode {
                    variable: OPTIMIZATIONS,
                    value,
                });
            }
        };
    }

    if let Some(value) = read_environment(CALL_DEPTH)? {
        configuration.call_depth_limit = parse_integer(CALL_DEPTH, value)?;
    }

    if let Some(value) = read_environment(CYCLE_THRESHOLD)? {
        configuration.cycle_threshold = Some(parse_integer(CYCLE_THRESHOLD, value)?);
    }

    if let Some(value) = read_environment(FULL_TRACE)? {
        configuration.full_trace =
            value
                .parse()
                .map_err(|source| Error::InvalidBooleanEnvironment {
                    variable: FULL_TRACE,
                    value,
                    source,
                })?;
    }

    Ok(configuration)
}

fn parse_integer(variable: &'static str, value: String) -> Result<usize, Error> {
    value
        .parse()
        .map_err(|source| Error::InvalidIntegerEnvironment {
            variable,
            value,
            source,
        })
}

fn read_environment(variable: &'static str) -> Result<Option<String>, Error> {
    match env::var(variable) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(source) => Err(Error::InvalidEnvironment { variable, source }),
    }
}
