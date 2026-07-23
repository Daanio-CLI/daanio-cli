pub const QUIET_ENV: &str = "DAANIO_QUIET";

pub fn set_quiet_enabled(enabled: bool) {
    if enabled {
        crate::env::set_var(QUIET_ENV, "1");
    } else {
        crate::env::remove_var(QUIET_ENV);
    }
}

pub fn quiet_enabled() -> bool {
    std::env::var(QUIET_ENV)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn stderr_info(message: impl AsRef<str>) {
    if !quiet_enabled() {
        eprintln!("{}", message.as_ref());
    }
}

pub fn stderr_blank_line() {
    if !quiet_enabled() {
        eprintln!();
    }
}

fn stderr_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("DAANIO_NO_COLOR").is_none()
        && crate::console::stderr_supports_ansi()
}

fn stderr_paint(text: impl AsRef<str>, code: &str) -> String {
    let text = text.as_ref();
    if stderr_color_enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn heading(text: impl AsRef<str>) -> String {
    stderr_paint(text, "1;36")
}

pub fn stderr_heading(text: impl AsRef<str>) {
    eprintln!("{}", heading(text));
}

pub fn success(text: impl AsRef<str>) -> String {
    stderr_paint(text, "1;32")
}

pub fn muted(text: impl AsRef<str>) -> String {
    stderr_paint(text, "2")
}

pub fn command(text: impl AsRef<str>) -> String {
    stderr_paint(text, "1;36")
}

pub fn link(text: impl AsRef<str>) -> String {
    stderr_paint(text, "4;36")
}
