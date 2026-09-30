//! Process-local startup compatibility; never change the user's desktop settings.
#[cfg(any(target_os = "linux", test))]
fn needs_wayland_override(display: Option<&str>, session: Option<&str>, explicit: bool) -> bool {
    !explicit && (display.is_some_and(|s| !s.is_empty()) || session == Some("wayland"))
}

pub fn configure_renderer() {
    #[cfg(target_os = "linux")]
    {
        let display = std::env::var("WAYLAND_DISPLAY").ok();
        let session = std::env::var("XDG_SESSION_TYPE").ok();
        if needs_wayland_override(
            display.as_deref(),
            session.as_deref(),
            std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_some(),
        ) {
            // Called from main before the monitor, runtime, or GTK starts threads.
            // Avoid the reproduced NVIDIA/Wayland Error 71 without disabling
            // WebKit's entire DMABUF renderer. Preserve explicit user overrides.
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_wayland_sessions_receive_the_compatibility_default() {
        assert!(needs_wayland_override(Some("wayland-0"), None, false));
        assert!(needs_wayland_override(None, Some("wayland"), false));
        assert!(!needs_wayland_override(None, Some("x11"), false));
        assert!(!needs_wayland_override(Some(""), None, false));
        assert!(!needs_wayland_override(
            Some("wayland-0"),
            Some("wayland"),
            true
        ));
    }
}
