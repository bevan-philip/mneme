use std::path::Path;

use anyhow::Context;
use anyhow::Result;

/// MathJax's entry point within `MATHJAX`, as pages load it.
pub const MATHJAX_SCRIPT: &str = "tex-chtml-full.js";

macro_rules! mathjax_files {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../vendor/mathjax/", $path)))),*]
    };
}

/// `vendor/mathjax/`, embedded so a preview typesets math offline: each file
/// at the path MathJax resolves it at, relative to its script.
const MATHJAX: &[(&str, &[u8])] = mathjax_files![
    "LICENSE",
    "tex-chtml-full.js",
    "output/chtml/fonts/woff-v2/MathJax_AMS-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Calligraphic-Bold.woff",
    "output/chtml/fonts/woff-v2/MathJax_Calligraphic-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Fraktur-Bold.woff",
    "output/chtml/fonts/woff-v2/MathJax_Fraktur-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Main-Bold.woff",
    "output/chtml/fonts/woff-v2/MathJax_Main-Italic.woff",
    "output/chtml/fonts/woff-v2/MathJax_Main-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Math-BoldItalic.woff",
    "output/chtml/fonts/woff-v2/MathJax_Math-Italic.woff",
    "output/chtml/fonts/woff-v2/MathJax_Math-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_SansSerif-Bold.woff",
    "output/chtml/fonts/woff-v2/MathJax_SansSerif-Italic.woff",
    "output/chtml/fonts/woff-v2/MathJax_SansSerif-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Script-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Size1-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Size2-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Size3-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Size4-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Typewriter-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Vector-Bold.woff",
    "output/chtml/fonts/woff-v2/MathJax_Vector-Regular.woff",
    "output/chtml/fonts/woff-v2/MathJax_Zero.woff",
];

/// Write the embedded MathJax into `dir`.
pub fn write_mathjax(dir: &Path) -> Result<()> {
    for (path, bytes) in MATHJAX {
        let path = dir.join(path);
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        std::fs::write(&path, bytes)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every font the script can ask for is embedded (MathJax requests them
    /// lazily, so a missing one only shows up as a broken glyph).
    #[test]
    fn embeds_every_font_the_script_references() {
        let script = MATHJAX
            .iter()
            .find(|(path, _)| *path == MATHJAX_SCRIPT)
            .unwrap()
            .1;
        let script = std::str::from_utf8(script).unwrap();
        let fonts = regex::Regex::new(r"MathJax_[A-Za-z0-9-]+\.woff").unwrap();
        let mut referenced = 0;
        for font in fonts.find_iter(script) {
            referenced += 1;
            let path = format!("output/chtml/fonts/woff-v2/{}", font.as_str());
            assert!(MATHJAX.iter().any(|(p, _)| *p == path), "{path}");
        }
        assert!(referenced > 0);
    }

    #[test]
    fn writes_the_script_where_pages_load_it() {
        let dir = tempfile::tempdir().unwrap();
        write_mathjax(dir.path()).unwrap();
        assert!(dir.path().join(MATHJAX_SCRIPT).is_file());
        assert!(
            dir.path()
                .join("output/chtml/fonts/woff-v2/MathJax_Zero.woff")
                .is_file()
        );
    }
}
