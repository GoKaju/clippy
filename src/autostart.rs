use std::path::PathBuf;

/// The full command line to re-launch the app (e.g. "clippy.exe connect 192.168.1.5:9876").
pub struct AutoStart {
    app_path: String,
    args: Vec<String>,
}

impl AutoStart {
    pub fn new(args: Vec<String>) -> Self {
        let app_path = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("clipboard-sync"))
            .to_string_lossy()
            .to_string();
        Self { app_path, args }
    }

    #[allow(dead_code)]
    fn full_command(&self) -> String {
        if self.args.is_empty() {
            format!("\"{}\"", self.app_path)
        } else {
            format!("\"{}\" {}", self.app_path, self.args.join(" "))
        }
    }

    pub fn is_enabled(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.plist_path().exists()
        }
        #[cfg(target_os = "windows")]
        {
            self.win_is_enabled()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    pub fn enable(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.mac_enable()
        }
        #[cfg(target_os = "windows")]
        {
            self.win_enable()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err("autostart not supported on this platform".into())
        }
    }

    pub fn disable(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.mac_disable()
        }
        #[cfg(target_os = "windows")]
        {
            self.win_disable()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err("autostart not supported on this platform".into())
        }
    }

    // ── macOS: LaunchAgent plist ──

    #[cfg(target_os = "macos")]
    fn plist_path(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join("Library/LaunchAgents/com.clippy.clipboard-sync.plist")
    }

    #[cfg(target_os = "macos")]
    fn mac_enable(&self) -> Result<(), String> {
        let plist_dir = self.plist_path().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&plist_dir).map_err(|e| e.to_string())?;

        let mut program_args = format!("    <string>{}</string>\n", self.app_path);
        for arg in &self.args {
            program_args.push_str(&format!("    <string>{}</string>\n", arg));
        }

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.clippy.clipboard-sync</string>
  <key>ProgramArguments</key>
  <array>
{}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
</dict>
</plist>
"#,
            program_args
        );

        std::fs::write(self.plist_path(), plist).map_err(|e| e.to_string())?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn mac_disable(&self) -> Result<(), String> {
        let path = self.plist_path();
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    // ── Windows: Registry Run key ──

    #[cfg(target_os = "windows")]
    fn win_is_enabled(&self) -> bool {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "Clippy",
            ])
            .output();
        matches!(output, Ok(o) if o.status.success())
    }

    #[cfg(target_os = "windows")]
    fn win_enable(&self) -> Result<(), String> {
        let cmd = self.full_command();
        let status = std::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "Clippy",
                "/t",
                "REG_SZ",
                "/d",
                &cmd,
                "/f",
            ])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("reg add failed".into())
        }
    }

    #[cfg(target_os = "windows")]
    fn win_disable(&self) -> Result<(), String> {
        let status = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "Clippy",
                "/f",
            ])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("reg delete failed".into())
        }
    }
}

/// Build an AutoStart with explicit path (for testing).
#[cfg(test)]
impl AutoStart {
    fn with_path(app_path: &str, args: Vec<String>) -> Self {
        Self {
            app_path: app_path.to_string(),
            args,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_command_no_args() {
        let a = AutoStart::with_path("/usr/bin/clippy", vec![]);
        assert_eq!(a.full_command(), "\"/usr/bin/clippy\"");
    }

    #[test]
    fn full_command_with_serve_args() {
        let a = AutoStart::with_path(
            "/usr/bin/clippy",
            vec!["serve".into(), "--port".into(), "9876".into()],
        );
        assert_eq!(a.full_command(), "\"/usr/bin/clippy\" serve --port 9876");
    }

    #[test]
    fn full_command_with_connect_args() {
        let a = AutoStart::with_path(
            "C:\\Program Files\\Clippy\\clippy.exe",
            vec!["connect".into(), "192.168.1.5:9876".into()],
        );
        assert_eq!(
            a.full_command(),
            "\"C:\\Program Files\\Clippy\\clippy.exe\" connect 192.168.1.5:9876"
        );
    }

    #[test]
    fn full_command_with_spaces_in_path() {
        let a = AutoStart::with_path("/path with spaces/clippy", vec!["serve".into()]);
        assert_eq!(a.full_command(), "\"/path with spaces/clippy\" serve");
    }

    #[test]
    fn new_gets_current_exe() {
        let a = AutoStart::new(vec!["serve".into()]);
        assert!(!a.app_path.is_empty());
        assert_eq!(a.args, vec!["serve"]);
    }
}
