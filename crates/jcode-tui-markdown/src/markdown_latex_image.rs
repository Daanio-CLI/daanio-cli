use image::GenericImageView;
use ratatui::prelude::Line;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const RENDERER_VERSION: u8 = 2;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const FOREGROUND: (u8, u8, u8) = (130, 210, 235);

#[derive(Debug, Clone)]
struct Toolchain {
    latex: PathBuf,
    dvipng: PathBuf,
    pdflatex: PathBuf,
    pdftocairo: PathBuf,
}

impl Toolchain {
    fn from_environment() -> Self {
        Self {
            latex: std::env::var_os("JCODE_LATEX_COMMAND")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("latex")),
            dvipng: std::env::var_os("JCODE_DVIPNG_COMMAND")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("dvipng")),
            pdflatex: std::env::var_os("JCODE_PDFLATEX_COMMAND")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("pdflatex")),
            pdftocairo: std::env::var_os("JCODE_PDFTOCAIRO_COMMAND")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("pdftocairo")),
        }
    }
}

pub(super) fn render_latex_image(
    source: &str,
    display: bool,
    max_width: Option<usize>,
) -> Result<Vec<Line<'static>>, String> {
    if !super::mermaid::image_protocol_available() {
        return Err("terminal image protocol unavailable".to_string());
    }
    let artifact = render_artifact(source, display, &Toolchain::from_environment())?;
    let hash =
        super::mermaid::register_external_image(&artifact.path, artifact.width, artifact.height);
    Ok(super::mermaid::result_to_lines(
        super::mermaid::RenderResult::Image {
            hash,
            path: artifact.path,
            width: artifact.width,
            height: artifact.height,
        },
        max_width,
    ))
}

#[derive(Debug)]
struct Artifact {
    path: PathBuf,
    width: u32,
    height: u32,
}

fn render_artifact(source: &str, display: bool, toolchain: &Toolchain) -> Result<Artifact, String> {
    render_artifact_in(source, display, toolchain, &cache_dir()?)
}

fn render_artifact_in(
    source: &str,
    display: bool,
    toolchain: &Toolchain,
    cache_dir: &Path,
) -> Result<Artifact, String> {
    validate_source(source)?;
    fs::create_dir_all(cache_dir).map_err(|e| format!("create LaTeX cache: {e}"))?;
    let cache_path = cache_dir.join(format!("{:016x}.png", cache_key(source, display)));
    if let Ok(artifact) = load_artifact(&cache_path) {
        return Ok(artifact);
    }

    let work = tempfile::Builder::new()
        .prefix("jcode-latex-")
        .tempdir()
        .map_err(|e| format!("create LaTeX workspace: {e}"))?;
    let tex_path = work.path().join("formula.tex");
    fs::write(&tex_path, latex_document(source, display))
        .map_err(|e| format!("write LaTeX source: {e}"))?;

    let dvi_result = run_command(
        &toolchain.latex,
        [
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "formula.tex",
        ],
        work.path(),
    )
    .and_then(|_| {
        run_command(
            &toolchain.dvipng,
            [
                "-D",
                "180",
                "-T",
                "tight",
                "-bg",
                "Transparent",
                "-fg",
                "rgb 130 210 235",
                "-o",
                "formula.png",
                "formula.dvi",
            ],
            work.path(),
        )
    });
    if let Err(dvi_error) = dvi_result {
        render_with_pdf_toolchain(toolchain, work.path()).map_err(|pdf_error| {
            format!("DVI renderer failed ({dvi_error}); PDF renderer failed ({pdf_error})")
        })?;
    }

    let rendered = work.path().join("formula.png");
    load_artifact(&rendered)?;
    let temporary_cache_path = cache_path.with_extension(format!("{}.tmp", std::process::id()));
    fs::copy(&rendered, &temporary_cache_path).map_err(|e| format!("cache rendered LaTeX: {e}"))?;
    if let Err(error) = fs::rename(&temporary_cache_path, &cache_path) {
        if !cache_path.exists() {
            let _ = fs::remove_file(&temporary_cache_path);
            return Err(format!("publish rendered LaTeX: {error}"));
        }
        let _ = fs::remove_file(&temporary_cache_path);
    }
    load_artifact(&cache_path)
}

fn render_with_pdf_toolchain(toolchain: &Toolchain, working_dir: &Path) -> Result<(), String> {
    run_command(
        &toolchain.pdflatex,
        [
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "formula.tex",
        ],
        working_dir,
    )?;
    run_command(
        &toolchain.pdftocairo,
        ["-png", "-singlefile", "-r", "180", "formula.pdf", "formula"],
        working_dir,
    )?;
    recolor_and_crop(&working_dir.join("formula.png"))
}

fn recolor_and_crop(path: &Path) -> Result<(), String> {
    let image = image::open(path)
        .map_err(|e| format!("read PDF-rendered LaTeX PNG: {e}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in image.enumerate_pixels() {
        let luminance =
            (u16::from(pixel[0]) * 54 + u16::from(pixel[1]) * 183 + u16::from(pixel[2]) * 19) / 256;
        if pixel[3] > 0 && luminance < 250 {
            bounds = Some(match bounds {
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
                None => (x, y, x, y),
            });
        }
    }
    let (min_x, min_y, max_x, max_y) =
        bounds.ok_or_else(|| "rendered formula is blank".to_string())?;
    let padding = 4;
    let left = min_x.saturating_sub(padding);
    let top = min_y.saturating_sub(padding);
    let right = max_x.saturating_add(padding).min(width.saturating_sub(1));
    let bottom = max_y.saturating_add(padding).min(height.saturating_sub(1));
    let mut cropped = image::imageops::crop_imm(
        &image,
        left,
        top,
        right.saturating_sub(left).saturating_add(1),
        bottom.saturating_sub(top).saturating_add(1),
    )
    .to_image();
    for pixel in cropped.pixels_mut() {
        let luminance =
            (u16::from(pixel[0]) * 54 + u16::from(pixel[1]) * 183 + u16::from(pixel[2]) * 19) / 256;
        let alpha = 255u16.saturating_sub(luminance) as u8;
        *pixel = image::Rgba([FOREGROUND.0, FOREGROUND.1, FOREGROUND.2, alpha]);
    }
    cropped
        .save(path)
        .map_err(|e| format!("write cropped LaTeX PNG: {e}"))
}

fn cache_dir() -> Result<PathBuf, String> {
    dirs::cache_dir()
        .map(|path| path.join("jcode").join("latex"))
        .ok_or_else(|| "no user cache directory is available".to_string())
}

fn load_artifact(path: &Path) -> Result<Artifact, String> {
    let image = image::open(path).map_err(|e| format!("read rendered LaTeX PNG: {e}"))?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("rendered LaTeX PNG is empty".to_string());
    }
    Ok(Artifact {
        path: path.to_path_buf(),
        width,
        height,
    })
}

fn run_command<const N: usize>(
    executable: &Path,
    args: [&str; N],
    working_dir: &Path,
) -> Result<(), String> {
    let mut child = Command::new(executable)
        .args(args)
        .current_dir(working_dir)
        .env("openin_any", "p")
        .env("openout_any", "p")
        .env("TEXMFOUTPUT", working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("start {}: {e}", executable.display()))?;
    match child
        .wait_timeout(COMMAND_TIMEOUT)
        .map_err(|e| format!("wait for {}: {e}", executable.display()))?
    {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(format!("{} exited with {status}", executable.display())),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("{} timed out", executable.display()))
        }
    }
}

fn cache_key(source: &str, display: bool) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    RENDERER_VERSION.hash(&mut hasher);
    source.hash(&mut hasher);
    display.hash(&mut hasher);
    FOREGROUND.hash(&mut hasher);
    hasher.finish()
}

fn latex_document(source: &str, display: bool) -> String {
    let math = if display {
        format!("\\[\\displaystyle\n{source}\n\\]")
    } else {
        format!("${source}$")
    };
    format!(
        "\\documentclass{{article}}\n\\pagestyle{{empty}}\n\\usepackage{{amsmath,amssymb}}\n\\begin{{document}}\n\\noindent {math}\n\\end{{document}}\n"
    )
}

fn validate_source(source: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err("LaTeX source is empty".to_string());
    }
    if source.len() > MAX_SOURCE_BYTES {
        return Err(format!("LaTeX source exceeds {MAX_SOURCE_BYTES} bytes"));
    }
    let lowered = source.to_ascii_lowercase();
    const FORBIDDEN: &[&str] = &[
        "\\input",
        "\\include",
        "\\openin",
        "\\openout",
        "\\read",
        "\\write",
        "\\immediate",
        "\\usepackage",
        "\\documentclass",
        "\\special",
        "\\catcode",
    ];
    if let Some(command) = FORBIDDEN.iter().find(|command| lowered.contains(**command)) {
        return Err(format!("unsafe LaTeX command is not allowed: {command}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_wraps_inline_and_display_math_without_shell_escape() {
        let inline = latex_document(r"x^2 + \\alpha", false);
        assert!(inline.contains(r"$x^2 + \\alpha$"));
        assert!(!inline.contains("shell-escape"));
        let display = latex_document(r"\\frac{a}{b}", true);
        assert!(display.contains("\\[\\displaystyle"));
        assert!(display.contains(r"\\frac{a}{b}"));
    }

    #[test]
    fn cache_key_is_stable_and_separates_inline_from_display() {
        assert_eq!(cache_key("x", false), cache_key("x", false));
        assert_ne!(cache_key("x", false), cache_key("x", true));
        assert_ne!(cache_key("x", false), cache_key("y", false));
    }

    #[test]
    fn unsafe_empty_and_oversized_sources_are_rejected() {
        assert!(validate_source(" ").is_err());
        assert!(validate_source(r"\\input{/etc/passwd}").is_err());
        assert!(validate_source(&"x".repeat(MAX_SOURCE_BYTES + 1)).is_err());
        assert!(validate_source(r"\\frac{x}{y}").is_ok());
    }

    #[test]
    fn missing_toolchain_returns_an_error_without_panicking() {
        let cache = tempfile::tempdir().unwrap();
        let missing = PathBuf::from("/definitely/missing/jcode-latex-command");
        let result = render_artifact_in(
            "unique_missing_toolchain_test_4815162342",
            false,
            &Toolchain {
                latex: missing.clone(),
                dvipng: missing.clone(),
                pdflatex: missing.clone(),
                pdftocairo: missing,
            },
            cache.path(),
        );
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn toolchain_output_is_validated_cached_and_reused() {
        use image::{ImageBuffer, Rgba};
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("fixture.png");
        ImageBuffer::from_pixel(7, 3, Rgba([130u8, 210, 235, 255]))
            .save(&fixture)
            .unwrap();
        let latex = root.path().join("latex-ok");
        let dvipng = root.path().join("dvipng-ok");
        fs::write(&latex, "#!/bin/sh\n: > formula.dvi\n").unwrap();
        fs::write(
            &dvipng,
            format!("#!/bin/sh\ncp '{}' formula.png\n", fixture.display()),
        )
        .unwrap();
        fs::set_permissions(&latex, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&dvipng, fs::Permissions::from_mode(0o755)).unwrap();
        let cache = root.path().join("cache");
        let artifact = render_artifact_in(
            r"\frac{x+1}{y}",
            true,
            &Toolchain {
                latex,
                dvipng,
                pdflatex: PathBuf::from("/unused/pdflatex"),
                pdftocairo: PathBuf::from("/unused/pdftocairo"),
            },
            &cache,
        )
        .unwrap();
        assert_eq!((artifact.width, artifact.height), (7, 3));
        assert!(artifact.path.starts_with(&cache));

        let missing = PathBuf::from("/definitely/missing/jcode-latex-command");
        let cached = render_artifact_in(
            r"\frac{x+1}{y}",
            true,
            &Toolchain {
                latex: missing.clone(),
                dvipng: missing.clone(),
                pdflatex: missing.clone(),
                pdftocairo: missing,
            },
            &cache,
        )
        .expect("the second render should use the validated PNG cache");
        assert_eq!((cached.width, cached.height), (7, 3));
        assert_eq!(cached.path, artifact.path);
    }

    #[cfg(unix)]
    #[test]
    fn pdf_fallback_crops_recolors_and_produces_a_cached_png() {
        use image::{ImageBuffer, Rgba};
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("pdf-page.png");
        let mut page = ImageBuffer::from_pixel(20, 10, Rgba([255u8, 255, 255, 255]));
        for y in 2..=6 {
            for x in 5..=10 {
                page.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        page.save(&fixture).unwrap();

        let failing_latex = root.path().join("latex-fail");
        let unused_dvipng = root.path().join("dvipng-unused");
        let pdflatex = root.path().join("pdflatex-ok");
        let pdftocairo = root.path().join("pdftocairo-ok");
        fs::write(&failing_latex, "#!/bin/sh\nexit 1\n").unwrap();
        fs::write(&unused_dvipng, "#!/bin/sh\nexit 99\n").unwrap();
        fs::write(&pdflatex, "#!/bin/sh\n: > formula.pdf\n").unwrap();
        fs::write(
            &pdftocairo,
            format!("#!/bin/sh\ncp '{}' formula.png\n", fixture.display()),
        )
        .unwrap();
        for path in [&failing_latex, &unused_dvipng, &pdflatex, &pdftocairo] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let artifact = render_artifact_in(
            r"x^2 + \alpha",
            false,
            &Toolchain {
                latex: failing_latex,
                dvipng: unused_dvipng,
                pdflatex,
                pdftocairo,
            },
            &root.path().join("cache"),
        )
        .unwrap();
        assert!(artifact.width < 20, "white page margins should be cropped");
        assert!(artifact.height <= 10);
        let rendered = image::open(&artifact.path).unwrap().into_rgba8();
        assert!(rendered.pixels().any(|pixel| pixel[3] == 255));
        assert!(rendered.pixels().all(|pixel| {
            [pixel[0], pixel[1], pixel[2]] == [FOREGROUND.0, FOREGROUND.1, FOREGROUND.2]
        }));
    }
}
