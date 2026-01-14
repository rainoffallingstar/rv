//! Conda environment management
//!
//! This module provides functionality to detect, query, and create conda/mamba/micromamba
//! environments for R package management.

use std::path::PathBuf;
use std::process::Command;

use thiserror::Error;
use which::which;

use crate::Version;
use crate::r_cmd::{RCommandLine, VersionError};

/// Conda tool types, prioritized by speed and efficiency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondaTool {
    /// Micromamba (fastest, no Python dependency)
    Micromamba,
    /// Mamba (fast, C++ implementation)
    Mamba,
    /// Conda (standard, slower)
    Conda,
}

impl CondaTool {
    /// Get the command name for the conda tool
    pub fn command(&self) -> &'static str {
        match self {
            CondaTool::Micromamba => "micromamba",
            CondaTool::Mamba => "mamba",
            CondaTool::Conda => "conda",
        }
    }
}

/// Information about a conda environment
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondaEnvironment {
    pub name: String,
    pub prefix: PathBuf,
    pub r_version: Version,
    pub r_lib: PathBuf,
    pub r_cmd: RCommandLine,
}

/// Error types for conda operations
#[derive(Error, Debug)]
pub enum CondaError {
    #[error("Tool not found")]
    ToolNotFound,

    #[error("Environment not found: {0}")]
    EnvironmentNotFound(String),

    #[error("Environment does not contain R: {0}")]
    NoRInEnvironment(String),

    #[error("Invalid R version: {0}")]
    InvalidRVersion(String),

    #[error("Failed to create environment: {0}")]
    CreateFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Version error: {0}")]
    Version(#[from] VersionError),
}

/// Conda environment manager
pub struct CondaManager {
    tool: Option<CondaTool>,
}

impl CondaManager {
    /// Create a new CondaManager with auto-detection of the best available tool
    pub fn new() -> Result<Self, CondaError> {
        let tool = Self::detect_tool()?;
        Ok(Self { tool })
    }

    /// Detect the best available conda tool (micromamba > mamba > conda)
    pub fn detect_tool() -> Result<Option<CondaTool>, CondaError> {
        // Try micromamba first (fastest, no Python dependency)
        if which("micromamba").is_ok() {
            return Ok(Some(CondaTool::Micromamba));
        }

        // Try mamba (faster than conda)
        if which("mamba").is_ok() {
            return Ok(Some(CondaTool::Mamba));
        }

        // Try conda (standard)
        if which("conda").is_ok() {
            return Ok(Some(CondaTool::Conda));
        }

        Ok(None)
    }

    /// Get information about a conda environment
    pub fn get_environment(&self, name: &str) -> Result<CondaEnvironment, CondaError> {
        let tool = self.tool.as_ref().ok_or(CondaError::ToolNotFound)?;

        // Check if environment exists by listing environments
        let output = Command::new(tool.command())
            .args(&["env", "list", "--json"])
            .output()
            .map_err(|e| CondaError::Io(e))?;

        if !output.status.success() {
            return Err(CondaError::EnvironmentNotFound(name.to_string()));
        }

        let envs: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;

        // Find the environment
        let envs_list = envs["envs"]
            .as_array()
            .ok_or_else(|| CondaError::EnvironmentNotFound(name.to_string()))?;

        let mut env_prefix = None;
        for env_path in envs_list {
            if let Some(env_path_str) = env_path.as_str() {
                // Check if this is the environment we're looking for
                let path_buf = PathBuf::from(env_path_str);

                // Check if it matches by name
                if path_buf
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s == name)
                    .unwrap_or(false)
                {
                    env_prefix = Some(path_buf);
                    break;
                }

                // Also check if the full path contains the name
                // Handle both Unix (/) and Windows (\) path separators
                let normalized_path = env_path_str.replace('\\', "/");
                if normalized_path.contains(&format!("/{}", name)) {
                    env_prefix = Some(path_buf);
                    break;
                }
            }
        }

        let prefix = env_prefix.ok_or_else(|| CondaError::EnvironmentNotFound(name.to_string()))?;

        // Get R version from the environment
        let r_version = self.get_r_version_from_environment(&prefix, tool)?;
        let r_lib = prefix.join("lib/R/library");

        // Find the full path to the conda executable
        let conda_path = which(tool.command()).ok();

        // Create RCommandLine for this environment
        let r_cmd = RCommandLine {
            r: None,
            conda_env: Some(name.to_string()),
            conda_path,
        };

        Ok(CondaEnvironment {
            name: name.to_string(),
            prefix,
            r_version,
            r_lib,
            r_cmd,
        })
    }

    /// Get R version from a conda environment
    fn get_r_version_from_environment(
        &self,
        prefix: &PathBuf,
        tool: &CondaTool,
    ) -> Result<Version, CondaError> {
        let output = Command::new(tool.command())
            .args(&["run", "-p", prefix.to_str().unwrap(), "R", "--version"])
            .output()
            .map_err(|e| CondaError::Io(e))?;

        log::debug!(
            "R --version command output: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        log::debug!(
            "R --version command stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        if !output.status.success() {
            // Try to find R binary directly in the environment
            let r_bin = prefix.join(if cfg!(windows) {
                "Scripts/R.exe"
            } else {
                "bin/R"
            });

            if r_bin.exists() {
                let output = Command::new(&r_bin)
                    .arg("--version")
                    .output()
                    .map_err(|e| CondaError::Io(e))?;

                if !output.status.success() {
                    return Err(CondaError::NoRInEnvironment(prefix.display().to_string()));
                }

                let version_str = String::from_utf8_lossy(&output.stdout);
                return crate::r_cmd::find_r_version(&version_str)
                    .ok_or_else(|| CondaError::InvalidRVersion(version_str.to_string()));
            }

            return Err(CondaError::NoRInEnvironment(prefix.display().to_string()));
        }

        let version_str = String::from_utf8_lossy(&output.stdout);
        let version = crate::r_cmd::find_r_version(&version_str)
            .ok_or_else(|| CondaError::InvalidRVersion(version_str.to_string()))?;

        Ok(version)
    }

    /// Create a new conda environment with R
    pub fn create_environment(
        &self,
        name: &str,
        r_version: &Version,
    ) -> Result<CondaEnvironment, CondaError> {
        let tool = self.tool.as_ref().ok_or(CondaError::ToolNotFound)?;

        log::debug!(
            "Creating conda environment '{}' with R version {}",
            name,
            r_version
        );

        // Build the create command
        let mut cmd = Command::new(tool.command());
        cmd.args(&["create", "-n", name, "-y"]);
        cmd.args(&["-c", "conda-forge"]);
        cmd.args(&["-c", "r"]);
        cmd.args(&["--strict-channel-priority"]);
        cmd.arg(&format!("r-base={}", r_version));

        // Execute the command
        let output = cmd.output().map_err(|e| CondaError::Io(e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CondaError::CreateFailed(stderr.to_string()));
        }

        // Verify the environment was created and get its info
        // If get_environment fails, construct it manually
        match self.get_environment(name) {
            Ok(env) => Ok(env),
            Err(e) => {
                log::warn!("Failed to get environment info after creation: {}", e);
                // Try to construct it manually
                if let Some(tool) = &self.tool {
                    let output = Command::new(tool.command())
                        .args(&["env", "list", "--json"])
                        .output()
                        .map_err(|e| CondaError::Io(e))?;

                    if output.status.success() {
                        let envs: serde_json::Value =
                            serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
                                .unwrap_or_default();
                        if let Some(envs_list) = envs["envs"].as_array() {
                            for env_path in envs_list {
                                if let Some(env_path_str) = env_path.as_str() {
                                    let path_buf = PathBuf::from(env_path_str);
                                    if path_buf
                                        .file_name()
                                        .and_then(|s| s.to_str())
                                        .map(|s| s == name)
                                        .unwrap_or(false)
                                    {
                                        let r_lib = path_buf.join("lib/R/library");
                                        let conda_path = which(tool.command()).ok();
                                        let r_cmd = RCommandLine {
                                            r: None,
                                            conda_env: Some(name.to_string()),
                                            conda_path,
                                        };
                                        return Ok(CondaEnvironment {
                                            name: name.to_string(),
                                            prefix: path_buf,
                                            r_version: r_version.clone(),
                                            r_lib,
                                            r_cmd,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e)
            }
        }
    }

    /// Check if an environment exists (only checks for existence, not R version)
    pub fn environment_exists(&self, name: &str) -> bool {
        let tool = match self.tool {
            Some(ref t) => t,
            None => return false,
        };

        // Check if environment exists by listing environments
        let output = match Command::new(tool.command())
            .args(&["env", "list", "--json"])
            .output()
        {
            Ok(o) => o,
            Err(_) => return false,
        };

        if !output.status.success() {
            return false;
        }

        let envs: Result<serde_json::Value, _> =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout));

        let envs = match envs {
            Ok(v) => v,
            Err(_) => return false,
        };

        let envs_list = match envs["envs"].as_array() {
            Some(a) => a,
            None => return false,
        };

        for env_path in envs_list {
            if let Some(env_path_str) = env_path.as_str() {
                let path_buf = PathBuf::from(env_path_str);

                // Check if it matches by name
                if path_buf
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s == name)
                    .unwrap_or(false)
                {
                    return true;
                }

                // Also check if the full path contains the name
                let normalized_path = env_path_str.replace('\\', "/");
                if normalized_path.contains(&format!("/{}", name)) {
                    return true;
                }
            }
        }

        false
    }

    /// List all conda environments
    pub fn list_environments(&self) -> Result<Vec<String>, CondaError> {
        let tool = self.tool.as_ref().ok_or(CondaError::ToolNotFound)?;

        let output = Command::new(tool.command())
            .args(&["env", "list", "--json"])
            .output()
            .map_err(|e| CondaError::Io(e))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let envs: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;

        let envs_list = envs["envs"]
            .as_array()
            .ok_or_else(|| CondaError::EnvironmentNotFound("".to_string()))?;

        let mut names = Vec::new();
        for env_path in envs_list {
            if let Some(env_path_str) = env_path.as_str() {
                let path_buf = PathBuf::from(env_path_str);
                if let Some(name) = path_buf.file_name().and_then(|s| s.to_str()) {
                    names.push(name.to_string());
                }
            }
        }

        Ok(names)
    }

    /// 在指定环境中安装包
    pub fn install_packages(
        &self,
        env_name: &str,
        packages: &[String],
        channels: Option<Vec<String>>,
    ) -> Result<(), CondaError> {
        let tool = self.tool.as_ref().ok_or(CondaError::ToolNotFound)?;

        log::info!(
            "Installing {} packages in conda environment '{}': {:?}",
            packages.len(),
            env_name,
            packages
        );

        let mut cmd = Command::new(tool.command());
        cmd.args(&["install", "-n", env_name, "-y"]);

        // 添加频道
        if let Some(chs) = channels {
            for ch in chs {
                cmd.args(&["-c", &ch]);
            }
        } else {
            // 默认使用 conda-forge
            cmd.args(&["-c", "conda-forge"]);
        }

        // 添加包名
        for pkg in packages {
            cmd.arg(pkg);
        }

        let output = cmd.output().map_err(|e| CondaError::Io(e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::error!("Failed to install conda packages: {}", stderr);
            return Err(CondaError::CreateFailed(stderr.to_string()));
        }

        log::info!("Successfully installed conda packages: {:?}", packages);
        Ok(())
    }

    /// 检查包是否在环境中已安装
    pub fn is_package_installed(
        &self,
        env_name: &str,
        package: &str,
    ) -> Result<bool, CondaError> {
        let tool = self.tool.as_ref().ok_or(CondaError::ToolNotFound)?;

        let output = Command::new(tool.command())
            .args(&["list", "--json", "-n", env_name])
            .output()
            .map_err(|e| CondaError::Io(e))?;

        if !output.status.success() {
            return Ok(false);
        }

        let list: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
                .map_err(|e| CondaError::Json(e))?;

        if let Some(packages) = list["packages"].as_array() {
            for pkg in packages {
                if let Some(name) = pkg["name"].as_str() {
                    if name == package {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conda_tool_command() {
        assert_eq!(CondaTool::Micromamba.command(), "micromamba");
        assert_eq!(CondaTool::Mamba.command(), "mamba");
        assert_eq!(CondaTool::Conda.command(), "conda");
    }

    #[test]
    fn test_detect_tool() {
        // This test will pass if any conda tool is available
        let result = CondaManager::detect_tool();
        // We can't assert the exact result since it depends on what's installed
        // But we can verify it returns a valid tool or None
        assert!(result.is_ok());
    }
}
