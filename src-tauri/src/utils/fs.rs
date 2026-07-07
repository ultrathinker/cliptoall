//! Small filesystem helpers shared across commands.

use std::path::Path;

/// Write `bytes` to `path` atomically: write a sibling temp file first, then
/// rename it over the target. A crash or power loss mid-write can then only ever
/// leave EITHER the previous file OR the temp file intact — never a truncated,
/// half-written target. For a config or token file a truncated write reads as
/// corruption (or, for settings, a silent reset to defaults), so every such
/// persist goes through here.
///
/// The temp file is created in the SAME directory as the target so the final
/// `rename` is a same-volume atomic move (a cross-volume rename would silently
/// degrade to a non-atomic copy). The temp name is randomized so two concurrent
/// writers to the same target can't clobber each other's temp file.
pub fn atomic_write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "target path has no parent directory".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "target path has no file name".to_string())?;
    let tmp = dir.join(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes.as_ref())
        .map_err(|e| format!("Failed to write temp file: {}", e))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp); // best-effort cleanup on failure
        return Err(format!("Failed to commit file: {}", e));
    }
    Ok(())
}
