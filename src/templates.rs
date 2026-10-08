//! Administrator-owned Handlebars templates with bounded output and a small cache.
use anyhow::{Context, Result, ensure};
use handlebars::Handlebars;
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

type Entry = (SystemTime, u64, Arc<Handlebars<'static>>);
static CACHE: OnceLock<Mutex<HashMap<PathBuf, Entry>>> = OnceLock::new();

fn load(path: &Path) -> Result<Option<Arc<Handlebars<'static>>>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && metadata.len() <= 1024 * 1024,
        "Template must be a file at most 1 MiB"
    );
    let modified = metadata.modified()?;
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((stamp, size, template)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(path)
        && *stamp == modified
        && *size == metadata.len()
    {
        return Ok(Some(template.clone()));
    }
    let source = std::fs::read_to_string(path)?;
    let mut engine = Handlebars::new();
    engine
        .register_template_string("page", source)
        .with_context(|| format!("Invalid template {}", path.display()))?;
    let engine = Arc::new(engine);
    let mut entries = cache.lock().unwrap_or_else(|p| p.into_inner());
    if entries.len() >= 256 {
        entries.clear();
    }
    entries.insert(path.to_owned(), (modified, metadata.len(), engine.clone()));
    Ok(Some(engine))
}
pub fn validate(directory: &Path) -> Result<()> {
    ensure!(
        directory.is_dir(),
        "Template directory does not exist: {}",
        directory.display()
    );
    let mut count = 0;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "html") {
            count += 1;
            ensure!(
                count <= 256,
                "Template directory contains more than 256 HTML files"
            );
            load(&path)?;
        }
    }
    Ok(())
}
struct Output(Vec<u8>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("Rendered template exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub async fn render(
    directory: &Path,
    name: &str,
    data: serde_json::Value,
) -> Result<Option<String>> {
    let mut paths = Vec::new();
    let mut candidate = name;
    loop {
        paths.push(directory.join(format!("{candidate}.html")));
        match candidate.rsplit_once('-') {
            Some((parent, _)) => candidate = parent,
            None => break,
        }
    }
    tokio::task::spawn_blocking(move || {
        for path in paths {
            if let Some(engine) = load(&path)? {
                let mut output = Output(Vec::new());
                engine.render_to_write("page", &data, &mut output)?;
                return Ok(Some(String::from_utf8(output.0)?));
            }
        }
        Ok(None)
    })
    .await?
}
