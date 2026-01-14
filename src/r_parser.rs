//! R 代码解析模块
//!
//! 此模块提供从 R 代码中提取包引用的功能

use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

use crate::consts::{BASE_PACKAGES, RECOMMENDED_PACKAGES};

// 正则表达式匹配 library(package) 或 library("package") 或 library('package')
fn library_re() -> Regex {
    // 匹配 library 后跟括号，包名可以有引号也可以没有
    Regex::new(r#"library\s*\(\s*['"]?([a-zA-Z][a-zA-Z0-9._]*)['"]?\s*\)"#).unwrap()
}

// 正则表达式匹配 require(package) 或 require("package") 或 require('package')
fn require_re() -> Regex {
    // 匹配 require 后跟括号，包名可以有引号也可以没有
    Regex::new(r#"require\s*\(\s*['"]?([a-zA-Z][a-zA-Z0-9._]*)['"]?\s*\)"#).unwrap()
}

// 正则表达式匹配 package::function 或 package:::internal
fn namespace_re() -> Regex {
    Regex::new(r#"([a-zA-Z][a-zA-Z0-9._]*)::"#).unwrap()
}

/// 从 R 代码内容中提取包名
///
/// # Examples
///
/// ```
/// let code = r#"
/// library(dplyr)
/// require(ggplot2)
/// data <- readr::read_csv("file.csv")
/// "#;
/// let packages = extract_packages_from_r_code(code);
/// assert!(packages.contains("dplyr"));
/// assert!(packages.contains("ggplot2"));
/// assert!(packages.contains("readr"));
/// ```
pub fn extract_packages_from_r_code(content: &str) -> HashSet<String> {
    let mut packages = HashSet::new();

    // 提取 library() 调用
    for cap in library_re().captures_iter(content) {
        if let Some(pkg) = cap.get(1) {
            packages.insert(pkg.as_str().to_string());
        }
    }

    // 提取 require() 调用
    for cap in require_re().captures_iter(content) {
        if let Some(pkg) = cap.get(1) {
            packages.insert(pkg.as_str().to_string());
        }
    }

    // 提取 package::function 形式
    for cap in namespace_re().captures_iter(content) {
        if let Some(pkg) = cap.get(1) {
            let pkg_name = pkg.as_str();
            // 过滤掉 R 关键字和内置函数
            if !is_r_keyword(pkg_name) {
                packages.insert(pkg_name.to_string());
            }
        }
    }

    // 过滤掉基础包和推荐包
    packages
        .into_iter()
        .filter(|pkg| !is_base_or_recommended(pkg))
        .collect()
}

/// 检查是否是 R 关键字
fn is_r_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "else" | "for" | "while" | "in" | "repeat" | "next" | "break"
            | "TRUE" | "FALSE" | "NULL" | "Inf" | "NaN" | "NA" | "NA_integer_"
            | "NA_real_" | "NA_character_" | "NA_complex_"
    )
}

/// 检查是否是基础包或推荐包
fn is_base_or_recommended(pkg: &str) -> bool {
    BASE_PACKAGES.contains(&pkg) || RECOMMENDED_PACKAGES.contains(&pkg)
}

/// 递归查找所有 .R 和 .r 文件
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// let files = find_r_files(Path::new("./src"));
/// assert!(!files.is_empty());
/// ```
pub fn find_r_files(dir: &Path) -> Vec<std::path::PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e
                    .path()
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("r"))
                    .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

/// 从单个 R 文件提取包名
pub fn extract_packages_from_r_file(
    file: &Path,
) -> Result<HashSet<String>, std::io::Error> {
    let content = std::fs::read_to_string(file)?;
    Ok(extract_packages_from_r_code(&content))
}

/// 从目录中所有 R 文件提取包名
///
/// 此函数会递归扫描目录中的所有 .R 和 .r 文件，
/// 并提取其中的包引用。如果某个文件读取失败，会打印警告但继续处理其他文件。
pub fn extract_packages_from_directory(dir: &Path) -> Result<HashSet<String>, std::io::Error> {
    let r_files = find_r_files(dir);
    let mut all_packages = HashSet::new();

    for file in r_files {
        match extract_packages_from_r_file(&file) {
            Ok(packages) => {
                all_packages.extend(packages);
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to read {}: {}",
                    file.display(),
                    e
                );
            }
        }
    }

    Ok(all_packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_library() {
        let code = r#"
            library(dplyr)
            library("ggplot2")
            library('tidyr')
        "#;
        let packages = extract_packages_from_r_code(code);
        assert!(packages.contains("dplyr"));
        assert!(packages.contains("ggplot2"));
        assert!(packages.contains("tidyr"));
    }

    #[test]
    fn test_extract_require() {
        let code = r#"
            require(data.table)
            require("custompkg")
        "#;
        let packages = extract_packages_from_r_code(code);
        assert!(packages.contains("data.table"));
        assert!(packages.contains("custompkg"));
    }

    #[test]
    fn test_extract_namespace() {
        let code = r#"
            df <- dplyr::select(mtcars, mpg)
            plot <- ggplot2::ggplot(data, aes(x = mpg))
            pkg::internal_func()
        "#;
        let packages = extract_packages_from_r_code(code);
        assert!(packages.contains("dplyr"));
        assert!(packages.contains("ggplot2"));
        assert!(packages.contains("pkg"));
    }

    #[test]
    fn test_filter_base_packages() {
        let code = r#"
            library(base)
            library(utils)
            library(dplyr)
        "#;
        let packages = extract_packages_from_r_code(code);
        // base 和 utils 应该被过滤掉
        assert!(!packages.contains("base"));
        assert!(!packages.contains("utils"));
        assert!(packages.contains("dplyr"));
    }

    #[test]
    fn test_filter_recommended_packages() {
        let code = r#"
            library(MASS)
            library(Matrix)
            library(custompkg)
        "#;
        let packages = extract_packages_from_r_code(code);
        // MASS 和 Matrix 是推荐包，应该被过滤掉
        assert!(!packages.contains("MASS"));
        assert!(!packages.contains("Matrix"));
        assert!(packages.contains("custompkg"));
    }

    #[test]
    fn test_filter_r_keywords() {
        let code = r#"
            # 这些是 R 关键字，不应该被识别为包
            if::else
            for::loop
            TRUE::FALSE
        "#;
        let packages = extract_packages_from_r_code(code);
        assert!(packages.is_empty());
    }

    #[test]
    fn test_mixed_patterns() {
        let code = r#"
            # Analysis script
            library(dplyr)
            library(ggplot2)

            data <- readr::read_csv("data.csv")
            plot <- ggplot2::ggplot(data, aes(x = mpg))

            require(reshape2)
        "#;
        let packages = extract_packages_from_r_code(code);
        assert_eq!(packages.len(), 4);
        assert!(packages.contains("dplyr"));
        assert!(packages.contains("ggplot2"));
        assert!(packages.contains("readr"));
        assert!(packages.contains("reshape2"));
    }

    #[test]
    fn test_no_duplicates() {
        let code = r#"
            library(dplyr)
            library(dplyr)
            dplyr::select()
            dplyr::filter()
        "#;
        let packages = extract_packages_from_r_code(code);
        // HashSet 自动去重
        assert_eq!(packages.len(), 1);
        assert!(packages.contains("dplyr"));
    }
}
