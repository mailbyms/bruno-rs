//! Git 集成：直接调用系统 `git`（原版 Bruno 用 dugite 内嵌 git，这里依赖 PATH 里的 git）。
//!
//! UI 实际用到的只有四件事：克隆仓库、列远端分支、读集合的 remote URL、连接/断开远端。
//! 连接/断开只改工作区配置（不碰仓库本身），与原版 `setCollectionGitRemote` 一致。

use std::path::Path;
use std::process::{Command, Output};

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 不弹黑框
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn run(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = git_command();
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let Output { status, stdout, stderr } = cmd.output().map_err(|e| format!("无法执行 git：{e}"))?;
    let out = String::from_utf8_lossy(&stdout).trim().to_string();
    let err = String::from_utf8_lossy(&stderr).trim().to_string();
    if status.success() {
        Ok(out)
    } else {
        Err(if err.is_empty() { format!("git {} 失败", args.join(" ")) } else { err })
    }
}

pub fn version() -> Option<String> {
    run(&["--version"], None).ok()
}

pub fn is_valid_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ssh://")
        || lower.starts_with("git://")
        || lower.starts_with("git@")
        // 本地仓库/bare 仓库也用 git 远程（file:// 或绝对路径），测试与内网场景常见
        || lower.starts_with("file://")
        || trimmed.starts_with('/')
        || trimmed.get(1..3).map(|s| s == ":\\" || s == ":/").unwrap_or(false)
}

pub fn clone_repo(url: &str, path: &str, branch: Option<&str>) -> Result<(), String> {
    if !is_valid_url(url) {
        return Err("Git 地址无效（支持 https://、ssh://、git@）".into());
    }
    let mut args: Vec<&str> = vec!["clone", "--depth", "1"];
    if let Some(b) = branch.filter(|b| !b.trim().is_empty()) {
        args.push("--branch");
        args.push(b);
    }
    args.push(url.trim());
    args.push(path);
    // 目录被创建了一半时清掉，避免留下坏仓库
    if let Err(e) = run(&args, None) {
        let target = crate::ops::to_path(path);
        let _ = std::fs::remove_dir_all(target);
        return Err(e);
    }
    Ok(())
}

/// 远端分支列表（git ls-remote --heads）
pub fn list_remote_branches(url: &str) -> Result<Vec<String>, String> {
    if !is_valid_url(url) {
        return Err("Git 地址无效".into());
    }
    let out = run(&["ls-remote", "--heads", url.trim()], None)?;
    let mut branches: Vec<String> = out
        .lines()
        .filter_map(|line| line.split('\t').nth(1))
        .filter_map(|r| r.strip_prefix("refs/heads/"))
        .map(|s| s.to_string())
        .collect();
    branches.sort();
    Ok(branches)
}

/// 集合目录的 origin 地址；没有仓库/没有远端时返回 None
pub fn collection_remote_url(path: &Path) -> Option<String> {
    if !path.is_dir() {
        return None;
    }
    run(&["config", "--get", "remote.origin.url"], Some(path))
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn is_repo(path: &Path) -> bool {
    path.is_dir() && path.join(".git").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dash_leading_urls() {
        assert!(!is_valid_url("--upload-pack=evil"));
        assert!(!is_valid_url(""));
        assert!(is_valid_url("https://github.com/a/b.git"));
        assert!(is_valid_url("git@github.com:a/b.git"));
    }

    #[test]
    fn reads_remote_url_when_repo_present() {
        let dir = std::env::temp_dir().join("brs-git-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if run(&["init"], Some(&dir)).is_err() {
            return; // 环境没有 git 时跳过
        }
        run(&["remote", "add", "origin", "https://example.com/a.git"], Some(&dir)).unwrap();
        assert_eq!(
            collection_remote_url(&dir).as_deref(),
            Some("https://example.com/a.git")
        );
        assert!(is_repo(&dir));
    }
}
