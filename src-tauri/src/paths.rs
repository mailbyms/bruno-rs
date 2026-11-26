//! 所有路径的唯一真相源。业务代码禁止硬编码 %APPDATA% 或其它应用目录。
//! 环境变量 BRUNO_RS_DATA_DIR 可覆盖，便于便携运行与隔离测试。

use std::env;
use std::path::PathBuf;

pub const DATA_DIR_ENV: &str = "BRUNO_RS_DATA_DIR";
const APP_DIR: &str = "bruno-rs";

fn platform_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR)
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
            .join(APP_DIR)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let base = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local")
                    .join("share")
            });
        base.join(APP_DIR)
    }
}

pub fn data_dir() -> PathBuf {
    env::var_os(DATA_DIR_ENV)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(platform_data_dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    pub root: PathBuf,
    pub settings: PathBuf,
    pub workspace: PathBuf,
    pub log: PathBuf,
    pub preferences: PathBuf,
    pub cookies: PathBuf,
    pub snapshot: PathBuf,
    pub global_environments: PathBuf,
}

pub fn layout(root: &PathBuf) -> DataLayout {
    DataLayout {
        root: root.clone(),
        settings: root.join("settings.json"),
        workspace: root.join("workspace.json"),
        log: root.join("startup.log"),
        preferences: root.join("preferences.json"),
        cookies: root.join("cookies.json"),
        snapshot: root.join("ui-snapshot.json"),
        global_environments: root.join("global-environments.json"),
    }
}
