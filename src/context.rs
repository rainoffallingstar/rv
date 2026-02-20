//! Project context for rv library usage

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use fs_err as fs;
#[cfg(feature = "cli")]
use rayon::prelude::*;
use serde::Deserialize;
use url::Url;

use crate::consts::{RUNIVERSE_PACKAGES_API_PATH, STAGING_DIR_NAME};

#[derive(Deserialize)]
struct Envs {
    environments: Vec<CondaEnvInfo>,
}

#[derive(Deserialize)]
struct CondaEnvInfo {
    name: Option<String>,
    prefix: PathBuf,
}
use crate::lockfile::Lockfile;
use crate::package::Package;
use crate::utils::create_spinner;
use crate::{
    Config, CondaManager, DiskCache, GitExecutor, Http, Library, RCommandLine, RCmd, Repository, RepositoryDatabase,
    Resolution, Resolver, SystemInfo, Version, find_r_version_command, get_package_file_urls, http,
    system_req,
};

/// Try to find the conda executable using multiple methods
/// Priority: micromamba > mamba > conda (regardless of CONDA_EXE env var)
fn find_conda_executable() -> Option<PathBuf> {
    // First, check for micromamba/mamba first (ignore CONDA_EXE which may point to slower conda)
    // This ensures we use the fastest available tool
    if let Ok(path) = which::which("micromamba") {
        log::debug!("Found micromamba via which: {}", path.display());
        return Some(path);
    }
    if let Ok(path) = which::which("mamba") {
        log::debug!("Found mamba via which: {}", path.display());
        return Some(path);
    }

    // Then check environment variables (only if micromamba/mamba not found)
    if let Ok(conda_exe) = env::var("CONDA_EXE") {
        let path = PathBuf::from(&conda_exe);
        if path.exists() {
            log::debug!("Found conda via CONDA_EXE: {}", path.display());
            return Some(path);
        }
    }

    // Second, try which() for conda (last resort)
    if let Ok(path) = which::which("conda") {
        log::debug!("Found conda via which: {}", path.display());
        return Some(path);
    }

    // Third, check common installation locations (priority: micromamba > mamba > conda)
    let home = env::var("HOME").ok()?;
    let common_locations = vec![
        // Micromamba (highest priority)
        format!("{}/micromamba/bin/micromamba", home),
        // Miniforge (includes mamba)
        format!("{}/miniforge3/condabin/mamba", home),
        format!("{}/miniforge3/bin/mamba", home),
        format!("{}/miniforge3/condabin/conda", home),
        format!("{}/miniforge3/bin/conda", home),
        // Miniconda
        format!("{}/miniconda3/condabin/conda", home),
        format!("{}/miniconda3/bin/conda", home),
        // Anaconda
        format!("{}/anaconda3/condabin/conda", home),
        format!("{}/anaconda3/bin/conda", home),
    ];

    for location in common_locations {
        let path = PathBuf::from(&location);
        if path.exists() {
            log::debug!("Found conda in common location: {}", path.display());
            return Some(path);
        }
    }

    log::debug!("Could not find conda executable");
    None
}

/// Method on how to find the R Version on the system
#[derive(Debug, Clone, PartialEq)]
pub enum RCommandLookup {
    /// Used for commands that require R to be on the system (installation commands)
    /// Also used for planning commands when the `--r-version` flag is not in use
    Strict,
    /// Used when the `--r-version` flag is set for planning commands
    Soft(Version),
    /// Used when finding the RCommand is not required, primarily for information commands like
    /// cache, library, etc.
    Skip,
}

impl From<Option<Version>> for RCommandLookup {
    /// convert Option<Version> to RCommandLookup, where if the Version is specified, it is a soft lookup
    /// If it is not specified, it is a strict lookup.
    fn from(ver: Option<Version>) -> Self {
        if let Some(v) = ver {
            Self::Soft(v)
        } else {
            Self::Strict
        }
    }
}

/// Mode for dependency resolution
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ResolveMode {
    /// Use lockfile if available and valid
    #[default]
    Default,
    /// Ignore lockfile and resolve all dependencies fresh
    FullUpgrade,
}

/// Project context containing all state needed for rv operations
#[derive(Debug)]
pub struct Context {
    pub config: Config,
    pub project_dir: PathBuf,
    pub r_version: Version,
    pub cache: DiskCache,
    pub library: Library,
    pub databases: Vec<(RepositoryDatabase, bool)>,
    pub lockfile: Option<Lockfile>,
    pub r_cmd: RCommandLine,
    pub builtin_packages: HashMap<String, Package>,
    /// Taken from posit API. Only for some linux distrib, it will remain empty
    /// on mac/windows/arch etc
    pub system_dependencies: HashMap<String, Vec<String>>,
    /// Whether to show progress bars/spinners
    pub show_progress_bar: bool,
    /// Conda environment information (if using conda)
    pub conda_env: Option<PathBuf>,
}

impl Context {
    pub fn new(
        config_file: &Path,
        r_command_lookup: RCommandLookup,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::new_with_cache_dir(config_file, r_command_lookup, None)
    }

    pub fn new_with_cache_dir(
        config_file: &Path,
        r_command_lookup: RCommandLookup,
        cache_dir: Option<&Path>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let config = Config::from_file(config_file)?;

        // Check if we're using a conda environment
        let conda_env_path = config.conda_env().map(PathBuf::from);

        // This can only be set to false if the user passed a r_version to rv plan
        let mut r_version_found = true;
        let (r_version, r_cmd) = match r_command_lookup {
            RCommandLookup::Strict => {
                if let Some(ref env_name) = conda_env_path {
                    // If using conda, detect the actual R version from the environment
                    let conda_path = find_conda_executable();
                    let r_cmd_line = RCommandLine {
                        conda_env: Some(env_name.to_string_lossy().to_string()),
                        r: None,
                        conda_path,
                    };
                    // Try to detect actual R version from conda environment
                    let actual_r_version = r_cmd_line.version().ok().map(|v| {
                        let version_str = format!("{}.{}", v.major_minor()[0], v.major_minor()[1]);
                        version_str.parse::<Version>().unwrap()
                    });
                    let r_version = actual_r_version.unwrap_or_else(|| config.r_version().clone());
                    (r_version, r_cmd_line)
                } else {
                    let r_version = config.r_version().clone();
                    let r_cmd = find_r_version_command(&r_version)?;
                    (r_version, r_cmd)
                }
            }
            RCommandLookup::Soft(v) => {
                let r_cmd = match find_r_version_command(&v) {
                    Ok(r) => r,
                    Err(_) => {
                        r_version_found = false;
                        RCommandLine::default()
                    }
                };
                let r_version = v.clone();
                (r_version, r_cmd)
            }
            RCommandLookup::Skip => {
                // Even when skipping R command lookup, we still need to respect conda_env configuration
                let r_version = config.r_version().clone();
                let r_cmd = if let Some(ref env_name) = conda_env_path {
                    // If using conda, set up the conda_env even for Skip mode
                    let conda_path = find_conda_executable();
                    RCommandLine {
                        conda_env: Some(env_name.to_string_lossy().to_string()),
                        r: None,
                        conda_path,
                    }
                } else {
                    // Without conda, we can't find R version in Skip mode
                    r_version_found = false;
                    RCommandLine::default()
                };
                (r_version, r_cmd)
            }
        };

        let cache = if let Some(dir) = cache_dir {
            DiskCache::new_in_dir(&r_version, SystemInfo::from_os_info(), dir)?
        } else {
            DiskCache::new(&r_version, SystemInfo::from_os_info())?
        };

        let project_dir = config_file.parent().unwrap().to_path_buf();
        let lockfile_path = project_dir.join(config.lockfile_name());
        let lockfile = if lockfile_path.exists() && config.use_lockfile() {
            if let Some(lockfile) = Lockfile::load(&lockfile_path)? {
                if !lockfile.r_version().hazy_match(&r_version) {
                    log::debug!(
                        "R version in config file and lockfile are not compatible. Ignoring lockfile."
                    );
                    None
                } else {
                    Some(lockfile)
                }
            } else {
                None
            }
        } else {
            None
        };

        // Determine the library path based on config or conda environment
        let library_path = if let Some(p) = config.library() {
            Some(p.clone())
        } else if let Some(ref conda_env) = conda_env_path {
            // Resolve conda environment to actual path
            let env_path = if let Ok(manager) = CondaManager::new() {
                if let Ok(env) = manager.get_environment(conda_env.to_string_lossy().as_ref()) {
                    env.prefix
                } else {
                    // Fallback: try to find environment using micromamba
                    let output = std::process::Command::new("micromamba")
                        .args(&["env", "list", "--json"])
                        .output();
                    let mut env_path = conda_env.clone();
                    if let Ok(output) = output {
                        if let Ok(Envs { environments }) = serde_json::from_str::<Envs>(&String::from_utf8_lossy(&output.stdout)) {
                            for env in environments {
                                if env.name.as_deref() == Some(conda_env.to_string_lossy().as_ref()) || env.prefix.file_name().map(|n| n.to_string_lossy() == conda_env.to_string_lossy()).unwrap_or(false) {
                                    env_path = env.prefix;
                                    break;
                                }
                            }
                        }
                    }
                    env_path
                }
            } else {
                conda_env.clone()
            };
            // Use the conda environment's R library path
            let lib_path = env_path.join("lib/R/library");
            log::debug!(
                "Using conda environment library path: {}",
                lib_path.display()
            );
            Some(lib_path)
        } else {
            None
        };

        let mut library = if let Some(p) = library_path {
            Library::new_custom(&project_dir, p)
        } else {
            Library::new(&project_dir, &cache.system_info, r_version.major_minor())
        };
        fs::create_dir_all(&library.path)?;
        log::debug!("Library path: {}", library.path.display());
        library.find_content();

        // We can only fetch the builtin packages if we have the right R
        let builtin_packages = if r_version_found {
            cache.get_builtin_packages_versions(&r_cmd)?
        } else {
            log::warn!(
                "R version not found: there may be issues with resolution regarding recommended packages"
            );
            HashMap::new()
        };

        Ok(Self {
            config,
            cache,
            r_version,
            project_dir,
            library,
            lockfile,
            databases: Vec::new(),
            r_cmd,
            builtin_packages,
            system_dependencies: HashMap::new(),
            show_progress_bar: false,
            conda_env: conda_env_path,
        })
    }

    /// Enable progress bar display for long-running operations
    pub fn show_progress_bar(&mut self) {
        self.show_progress_bar = true;
    }

    /// Load package databases from repositories
    pub fn load_databases(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let pb = create_spinner(self.show_progress_bar, "Loading databases...");
        self.databases = load_databases(self.config.repositories(), &self.cache)?;
        pb.finish_and_clear();
        Ok(())
    }

    /// Load databases only if the lockfile cannot fully resolve dependencies
    pub fn load_databases_if_needed(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let can_resolve = self
            .lockfile
            .as_ref()
            .map(|l| l.can_resolve(self.config.dependencies(), self.config.repositories()))
            .unwrap_or(false);

        if !can_resolve {
            self.load_databases()?;
        }
        Ok(())
    }

    /// Load system requirements from posit API (only supported on some Linux distros)
    pub fn load_system_requirements(&mut self) {
        if !system_req::is_supported(&self.cache.system_info) {
            return;
        }
        let pb = create_spinner(self.show_progress_bar, "Loading system requirements...");
        self.system_dependencies = self.cache.get_system_requirements();
        pb.finish_and_clear();
    }

    /// Load databases and system requirements based on resolve mode
    pub fn load_for_resolve_mode(
        &mut self,
        resolve_mode: ResolveMode,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // If the sync mode is an upgrade, we want to load the databases even if all packages
        // are contained in the lockfile because we ignore the lockfile during initial resolution
        match resolve_mode {
            ResolveMode::Default => self.load_databases_if_needed()?,
            ResolveMode::FullUpgrade => self.load_databases()?,
        }
        self.load_system_requirements();
        Ok(())
    }

    pub fn lockfile_path(&self) -> PathBuf {
        self.project_dir.join(self.config.lockfile_name())
    }

    pub fn library_path(&self) -> &Path {
        self.library.path()
    }

    pub fn staging_path(&self) -> PathBuf {
        self.library.path.join(STAGING_DIR_NAME)
    }

    pub fn resolve(&self, resolve_mode: ResolveMode) -> Resolution<'_> {
        let lockfile = match resolve_mode {
            ResolveMode::Default => &self.lockfile,
            ResolveMode::FullUpgrade => &None,
        };

        let mut resolver = Resolver::new(
            &self.project_dir,
            &self.databases,
            self.config.repositories().iter().map(|x| x.url()).collect(),
            &self.r_version,
            &self.builtin_packages,
            lockfile.as_ref(),
            self.config.packages_env_vars(),
        );

        if self.show_progress_bar {
            resolver.show_progress_bar();
        }

        let mut resolution = resolver.resolve(
            self.config.dependencies(),
            self.config.prefer_repositories_for(),
            &self.cache,
            &GitExecutor {},
            &Http {},
        );

        // If upgrade mode and there is a lockfile, adjust from_lockfile flags
        // to indicate which resolved deps match what was in the lockfile
        if resolve_mode == ResolveMode::FullUpgrade && self.lockfile.is_some() {
            resolution.found = resolution
                .found
                .into_iter()
                .map(|mut dep| {
                    dep.from_lockfile = self.lockfile.as_ref().unwrap().contains_resolved_dep(&dep);
                    dep
                })
                .collect::<Vec<_>>();
        }

        resolution
    }
}

/// Load package databases from repositories
/// Uses parallel iteration when cli feature is enabled, sequential otherwise
pub fn load_databases(
    repositories: &[Repository],
    cache: &DiskCache,
) -> Result<Vec<(RepositoryDatabase, bool)>, Box<dyn Error + Send + Sync>> {
    #[cfg(feature = "cli")]
    let iter = repositories.par_iter();
    #[cfg(not(feature = "cli"))]
    let iter = repositories.iter();

    let results: Vec<Result<_, Box<dyn Error + Send + Sync>>> = iter
        .map(|r| {
            let db = load_single_database(r, cache)?;
            Ok((db, r.force_source))
        })
        .collect();

    // Collect results, returning first error if any
    let mut dbs = Vec::with_capacity(results.len());
    for result in results {
        dbs.push(result?);
    }
    Ok(dbs)
}

fn load_single_database(
    r: &Repository,
    cache: &DiskCache,
) -> Result<RepositoryDatabase, Box<dyn Error + Send + Sync>> {
    // 1. Generate path to add to URL to get the src PACKAGE and binary PACKAGE for current OS
    let (path, exists) = cache.get_package_db_entry(r.url());

    // 2. Check in cache whether we have the database and is not expired
    if exists {
        // load the archive
        // We want to fallback on fetching it again if we somehow can't load it
        if let Ok(db) = RepositoryDatabase::load(&path) {
            log::debug!("Loaded packages db from {path:?}");
            return Ok(db);
        } else {
            log::debug!("Failed to load packages db from {path:?}");
        }
    }

    if r.url().contains("r-universe.dev") {
        if path.exists() {
            fs::remove_file(&path)?;
        }
        log::debug!("Need to download R-Universe packages API for {}", r.url());
        let mut db = RepositoryDatabase::new(r.url());
        let mut r_universe_api = Vec::new();
        let api_url = format!("{}/{RUNIVERSE_PACKAGES_API_PATH}", r.url())
            .parse::<Url>()
            .map_err(|e| format!("Invalid URL: {e}"))?;

        let bytes_read = http::download(&api_url, &mut r_universe_api, Vec::new())?;

        if bytes_read == 0 {
            return Err(format!("File at {api_url} was not found").into());
        }

        db.parse_runiverse_api(&String::from_utf8_lossy(&r_universe_api));

        db.persist(&path)?;
        log::debug!("Saving packages db at {path:?}");
        Ok(db)
    } else {
        // Make sure to remove the file if it exists - it's expired
        if path.exists() {
            fs::remove_file(&path)?;
        }
        log::debug!("Need to download PACKAGES file for {}", r.url());
        let mut db = RepositoryDatabase::new(r.url());
        // download files, parse them and persist to disk
        let mut source_package = Vec::new();
        let (source_url, binary_url) = get_package_file_urls(
            &Url::parse(r.url()).map_err(|e| format!("Invalid URL: {e}"))?,
            &cache.r_version,
            &cache.system_info,
        );

        let bytes_read = http::download(&source_url, &mut source_package, Vec::new())?;

        // We should ALWAYS have a PACKAGES file for source
        if bytes_read == 0 {
            return Err(format!("File at {source_url} was not found").into());
        }
        // UNSAFE: we trust the PACKAGES data to be valid UTF-8
        db.parse_source(unsafe { std::str::from_utf8_unchecked(&source_package) });

        let mut binary_package = Vec::new();
        // we do not know for certain that the Some return of get_binary_path will be a valid url,
        // but we do know that if it returns None there is not a binary PACKAGES file
        if let Some(url) = binary_url {
            log::debug!("checking for binary packages URL: {url}");
            let bytes_read = http::download(&url, &mut binary_package, vec![]).unwrap_or(0);
            // but sometimes we might not have a binary PACKAGES file and that's fine.
            // We only load binary if we found a file
            if bytes_read > 0 {
                // UNSAFE: we trust the PACKAGES data to be valid UTF-8
                db.parse_binary(
                    unsafe { std::str::from_utf8_unchecked(&binary_package) },
                    cache.r_version,
                );
            }
        } else {
            log::debug!("No binary URL.")
        }

        db.persist(&path)?;
        log::debug!("Saving packages db at {path:?}");
        Ok(db)
    }
}
