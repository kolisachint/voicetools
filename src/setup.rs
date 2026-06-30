//! First-run setup: resolve a model name, report what's installed, and
//! download pre-exported model files from HuggingFace.
//!
//! Pre-exported int8 ONNX files are published on HuggingFace, so no NeMo /
//! Python toolchain is needed — `voicetools setup` just fetches them into the
//! per-user data directory.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

/// Which backend a model drives. Lets `main` pick the right loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Parakeet,
    Whisper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Recommended — 25 European languages. PalatineVision/parakeet-tdt-0.6b-v3-onnx
    ParakeetV3Int8,
    /// English-only. istupakov/parakeet-tdt-0.6b-v2-onnx
    ParakeetV2Int8,
    /// Fallback — whisper.cpp ggml small.en
    WhisperSmallEn,
}

impl Model {
    /// Parse a CLI model name. Accepts a few friendly aliases.
    pub fn parse(name: &str) -> anyhow::Result<Model> {
        match name.trim().to_lowercase().as_str() {
            "parakeet" | "parakeet-v3" | "v3" => Ok(Model::ParakeetV3Int8),
            "parakeet-v2" | "v2" => Ok(Model::ParakeetV2Int8),
            "whisper" | "whisper-small" | "whisper-small-en" => Ok(Model::WhisperSmallEn),
            other => Err(anyhow!(
                "unknown model '{other}' (try: parakeet-v3, parakeet-v2, whisper-small)"
            )),
        }
    }

    /// Stable identifier used as the on-disk directory name.
    pub fn id(&self) -> &'static str {
        match self {
            Model::ParakeetV3Int8 => "parakeet-v3",
            Model::ParakeetV2Int8 => "parakeet-v2",
            Model::WhisperSmallEn => "whisper-small",
        }
    }

    pub fn backend(&self) -> Backend {
        match self {
            Model::ParakeetV3Int8 | Model::ParakeetV2Int8 => Backend::Parakeet,
            Model::WhisperSmallEn => Backend::Whisper,
        }
    }

    pub fn size_hint(&self) -> &'static str {
        match self {
            Model::ParakeetV3Int8 => "~650MB",
            Model::ParakeetV2Int8 => "~631MB",
            Model::WhisperSmallEn => "244MB",
        }
    }

    /// `(remote_url, local_filename)` pairs to fetch. Local filenames are
    /// canonicalized so every Parakeet variant loads through the same code.
    pub fn files(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Model::ParakeetV3Int8 => vec![
                ("https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx", "encoder.int8.onnx"),
                ("https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx", "decoder_joint.int8.onnx"),
                ("https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx", "nemo128.onnx"),
                ("https://huggingface.co/PalatineVision/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt", "vocab.txt"),
            ],
            Model::ParakeetV2Int8 => vec![
                ("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.int8.onnx", "encoder.int8.onnx"),
                ("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/decoder_joint-model.int8.onnx", "decoder_joint.int8.onnx"),
                ("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/nemo128.onnx", "nemo128.onnx"),
                ("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/vocab.txt", "vocab.txt"),
            ],
            Model::WhisperSmallEn => vec![(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
                "ggml-small.en.bin",
            )],
        }
    }

    /// Directory this model is (or will be) installed in.
    pub fn dir(&self) -> anyhow::Result<PathBuf> {
        Ok(models_root()?.join(self.id()))
    }

    /// Whether every file for this model is present on disk.
    pub fn is_ready(&self) -> bool {
        match self.dir() {
            Ok(dir) => self
                .files()
                .iter()
                .all(|(_, name)| dir.join(name).exists()),
            Err(_) => false,
        }
    }
}

/// Root directory holding all installed models.
pub fn models_root() -> anyhow::Result<PathBuf> {
    let base = dirs::data_dir()
        .context("could not determine a data directory for this platform")?;
    Ok(base.join("voicetools").join("models"))
}

/// `voicetools setup --model <name>`: download a model if it isn't present.
pub fn run(name: &str) -> anyhow::Result<()> {
    let model = Model::parse(name)?;
    let dir = model.dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create model dir {}", dir.display()))?;

    eprintln!(
        "Setting up {} ({}) in {}",
        model.id(),
        model.size_hint(),
        dir.display()
    );

    for (url, filename) in model.files() {
        let dest = dir.join(filename);
        if dest.exists() {
            eprintln!("  ✓ {filename} (already present)");
            continue;
        }
        eprintln!("  ↓ {filename}");
        download_with_progress(url, &dest)
            .with_context(|| format!("downloading {url}"))?;
    }

    eprintln!("Done. {} is ready.", model.id());
    crate::protocol::status("ready");
    Ok(())
}

/// `voicetools models`: list every known model and whether it's installed.
pub fn list() -> anyhow::Result<()> {
    let root = models_root()?;
    eprintln!("Models directory: {}", root.display());
    for model in [
        Model::ParakeetV3Int8,
        Model::ParakeetV2Int8,
        Model::WhisperSmallEn,
    ] {
        let mark = if model.is_ready() { "installed" } else { "not installed" };
        println!("{:<14} {:<8} [{}]", model.id(), model.size_hint(), mark);
    }
    Ok(())
}

/// Stream `url` to `dest`, writing to a `.part` file first and renaming on
/// success so a partial download never looks complete. Progress is printed to
/// **stderr** to keep stdout clean for the line protocol.
pub fn download_with_progress(url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = ureq::get(url)
        .set("User-Agent", "voicetools/0.1")
        .call()
        .with_context(|| format!("request failed: {url}"))?;

    let total: Option<u64> = resp
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok());

    let tmp = dest.with_extension("part");
    let mut reader = resp.into_reader();
    let mut file = fs::File::create(&tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;

    let mut written = 0u64;
    let mut last_print = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        written += n as u64;
        // Refresh the progress line roughly every 5 MB.
        if written - last_print >= 5 * 1024 * 1024 {
            last_print = written;
            match total {
                Some(t) if t > 0 => {
                    eprint!("\r    {:>3.0}%  ({} / {})", written as f64 / t as f64 * 100.0, human(written), human(t));
                }
                _ => eprint!("\r    {}", human(written)),
            }
            let _ = std::io::stderr().flush();
        }
    }
    file.flush()?;
    drop(file);
    eprintln!("\r    100%  ({})        ", human(written));

    fs::rename(&tmp, dest)
        .with_context(|| format!("finalizing {}", dest.display()))?;
    Ok(())
}

/// Render a byte count in human-friendly units.
fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases() {
        assert_eq!(Model::parse("parakeet").unwrap(), Model::ParakeetV3Int8);
        assert_eq!(Model::parse("V3").unwrap(), Model::ParakeetV3Int8);
        assert_eq!(Model::parse("parakeet-v2").unwrap(), Model::ParakeetV2Int8);
        assert_eq!(Model::parse("whisper").unwrap(), Model::WhisperSmallEn);
        assert!(Model::parse("nope").is_err());
    }

    #[test]
    fn backends_match_models() {
        assert_eq!(Model::ParakeetV3Int8.backend(), Backend::Parakeet);
        assert_eq!(Model::WhisperSmallEn.backend(), Backend::Whisper);
    }

    #[test]
    fn parakeet_downloads_four_files() {
        assert_eq!(Model::ParakeetV3Int8.files().len(), 4);
    }

    #[test]
    fn human_units() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2.0 KB");
        assert_eq!(human(5 * 1024 * 1024), "5.0 MB");
    }
}
