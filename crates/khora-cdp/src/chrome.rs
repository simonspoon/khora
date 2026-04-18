use khora_core::error::{KhoraError, KhoraResult};
use std::path::PathBuf;

/// Discover the Chrome/Chromium binary path for the current platform.
///
/// Search order:
/// 1. `CHROME_PATH` environment variable
/// 2. Platform-specific well-known locations
/// 3. `which` lookup for common binary names
pub fn find_chrome() -> KhoraResult<PathBuf> {
    // Check env var first
    if let Ok(path) = std::env::var("CHROME_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
        tracing::warn!("CHROME_PATH={path} does not exist, searching system paths");
    }

    // Platform-specific paths
    if let Some(path) = platform_chrome_path() {
        return Ok(path);
    }

    // Fallback to which
    for name in chrome_binary_names() {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    Err(KhoraError::ChromeNotFound)
}

/// Platform-specific Chrome locations.
#[cfg(target_os = "macos")]
fn platform_chrome_path() -> Option<PathBuf> {
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(target_os = "linux")]
fn platform_chrome_path() -> Option<PathBuf> {
    let candidates = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(target_os = "windows")]
fn platform_chrome_path() -> Option<PathBuf> {
    let program_files = [
        std::env::var("ProgramFiles").unwrap_or_default(),
        std::env::var("ProgramFiles(x86)").unwrap_or_default(),
        std::env::var("LocalAppData").unwrap_or_default(),
    ];
    let suffixes = [
        r"Google\Chrome\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ];
    for base in &program_files {
        if base.is_empty() {
            continue;
        }
        for suffix in &suffixes {
            let p = PathBuf::from(base).join(suffix);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_chrome_path() -> Option<PathBuf> {
    None
}

/// Common Chrome binary names for `which` lookup.
fn chrome_binary_names() -> &'static [&'static str] {
    &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests that mutate CHROME_PATH to avoid parallel races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_find_chrome_respects_env_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Test with a path that exists on all platforms
        let test_path = std::env::current_exe().unwrap();
        std::env::set_var("CHROME_PATH", &test_path);
        let result = find_chrome();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_path);
        std::env::remove_var("CHROME_PATH");
    }

    #[test]
    fn test_find_chrome_ignores_nonexistent_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CHROME_PATH", "/nonexistent/chrome/path/xyz");
        let result = find_chrome();
        // Should either find system Chrome or return ChromeNotFound
        // The important thing is it didn't use the nonexistent path
        if let Ok(path) = &result {
            assert_ne!(*path, PathBuf::from("/nonexistent/chrome/path/xyz"));
        }
        std::env::remove_var("CHROME_PATH");
    }

    #[test]
    fn test_chrome_binary_names_not_empty() {
        assert!(!chrome_binary_names().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_platform_chrome_path_macos() {
        // On macOS CI/dev machines, Chrome is usually installed
        // Just verify the function doesn't panic
        let _result = platform_chrome_path();
    }
}
