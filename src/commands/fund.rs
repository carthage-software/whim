use crate::config::Configuration;
use crate::error::Error;
use crate::output;
use crate::package::Project;
use crate::package::Source;

#[tracing::instrument(level = "debug", skip_all)]
pub(super) fn execute(configuration: &Configuration) -> Result<(), Error> {
    let project = Project::inspect(configuration)?;
    let manifest = project.manifest();
    let Some(lock) = project.current_lock(manifest)? else {
        output::write("Whim\n  https://github.com/azjezz\n")?;
        return Ok(());
    };

    let mut report = String::from("Whim\n  https://github.com/azjezz\n");
    for package in &lock.packages {
        let Some(sponsor) = &package.sponsor else {
            continue;
        };

        let source = Source::parse(&package.source)?;
        if !project.is_installed(&source)? {
            continue;
        }

        report.push('\n');
        report.push_str(&package.source);
        report.push(' ');
        report.push_str(&package.version.to_string());
        report.push_str("\n  ");
        report.push_str(sponsor);
        report.push('\n');
    }

    output::write(&report)?;
    Ok(())
}
