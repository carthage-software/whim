use clap::ColorChoice;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

use crate::color::should_use_colors;
use crate::error::Error;

pub(crate) fn create(
    mut configuration: EngineConfiguration,
    colors: ColorChoice,
) -> Result<Engine, Error> {
    let diagnostic_color = should_use_colors(colors);
    let mut engine = Engine::new(EngineConfiguration {
        diagnostic_color,
        ..EngineConfiguration::default()
    });

    whim_lib::load(&mut engine).map_err(Error::LoadStandardLibrary)?;

    configuration.diagnostic_color = diagnostic_color;
    engine.configure(configuration);
    Ok(engine)
}

#[cfg(test)]
mod tests {
    use clap::ColorChoice;
    use whim_runtime::engine::EngineConfiguration;

    use super::create;

    #[test]
    fn runtime_call_depth_does_not_limit_standard_library_loading() {
        let configuration = EngineConfiguration {
            call_depth_limit: 2,
            ..EngineConfiguration::default()
        };

        assert!(create(configuration, ColorChoice::Never).is_ok());
    }
}
