use std::{
    fs::File,
    io::Write,
    path::{Path, absolute},
};

use anyhow::{Result, anyhow};

use crate::{
    DiskCache, RenvLock, Repository, SystemInfo, Context,
    context::load_databases,
    renv::{ResolvedRenv, UnresolvedRenv},
};

const RENV_CONFIG_TEMPLATE: &str = r#"# this config was migrated from %renv_file% on %time%
[project]
name = "%project_name%"
r_version = "%r_version%"
%conda_env%
repositories = [
%repositories%
]

dependencies = [
%dependencies%
]
"#;

pub fn migrate_renv(
    renv_file: impl AsRef<Path>,
    config_file: impl AsRef<Path>,
    strict_r_version: bool,
    context: &Context,
) -> Result<Vec<UnresolvedRenv>> {
    // project name is the parent directory of the renv project
    let abs_renv_file = absolute(renv_file.as_ref())?;
    let project_name = abs_renv_file
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("renv migrated project");

    // use the repositories and r version from the renv.lock to determine the repository databases
    let renv_lock = RenvLock::parse_renv_lock(&renv_file)?;
    let cache = match DiskCache::new(renv_lock.r_version(), SystemInfo::from_os_info()) {
        Ok(c) => c,
        Err(e) => return Err(anyhow!(e)),
    };
    let databases =
        load_databases(&renv_lock.config_repositories(), &cache).map_err(|e| anyhow!("{e}"))?;

    // resolve the renv.lock file to determine the true source of packages
    let (resolved, unresolved) = renv_lock.resolve(&databases);

    // Write config out to the config file specified in the cli, even if config file is outside of the renv.lock project
    let r_version = if strict_r_version {
        &renv_lock.r_version().original
    } else {
        let [major, minor] = renv_lock.r_version().major_minor();
        &format!("{major}.{minor}")
    };

    let config = render_config(
        &renv_file.as_ref().to_string_lossy(),
        project_name,
        r_version,
        &renv_lock.config_repositories(),
        &resolved,
        context,
    );
    let mut file = File::create(&config_file)?;
    file.write_all(config.as_bytes())?;
    Ok(unresolved)
}

fn render_config(
    renv_file: &str,
    project_name: &str,
    r_version: &str,
    repositories: &[Repository],
    resolved_deps: &[ResolvedRenv],
    context: &Context,
) -> String {
    let repos = repositories
        .iter()
        .map(|r| {
            format!(
                r#"    {{ alias = "{}", url = "{}"{}}}"#,
                r.alias,
                r.url(),
                if r.force_source {
                    r#", force_source = true"#.to_string()
                } else {
                    String::new()
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    // print alphabetically to match with plan/sync output
    let deps = resolved_deps
        .iter()
        .map(|d| format!("    {d}"))
        .collect::<Vec<_>>()
        .join(",\n");

    // Add conda_env if configured
    let conda_env_line = if let Some(ref conda_env) = context.conda_env {
        format!("conda_env = \"{}\"", conda_env.display())
    } else {
        String::new()
    };

    // get time. Try to round to seconds, but if error, leave as unrounded
    let time = jiff::Zoned::now();
    // Format the time as just the date (YYYY-MM-DD)
    let time = time.date().to_string();

    RENV_CONFIG_TEMPLATE
        .replace("%renv_file%", renv_file)
        .replace("%time%", &time.to_string())
        .replace("%project_name%", project_name)
        .replace("%r_version%", r_version)
        .replace("%conda_env%", &if conda_env_line.is_empty() {
            String::new()
        } else {
            format!("{}\n", conda_env_line)
        })
        .replace("%repositories%", &repos)
        .replace("%dependencies%", &deps)
}
