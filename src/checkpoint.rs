//! Fail closed around every externally visible send.
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("pending")
}

fn require_clear(path: &Path) -> io::Result<()> {
    match std::fs::metadata(pending_path(path)) {
        Ok(_) => Err(io::Error::other(
            "Unconfirmed delivery pending; reconcile before retrying",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn helper(path: &Path, operation: &str, identity: Option<&str>) -> io::Result<()> {
    if let Some(helper) = std::env::var_os("DELIVERY_CHECKPOINT") {
        let mut command = Command::new("python3");
        command.arg(helper).arg(operation).arg(path);
        if let Some(identity) = identity {
            command.arg(identity);
        }
        if !command.status()?.success() {
            return Err(io::Error::other(
                "Remote delivery checkpoint failed; stopping sends",
            ));
        }
    }
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("checkpoint-tmp");
    let mut file = std::fs::File::create(&temporary)?;
    use std::io::Write;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
}

pub fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    require_clear(path)?;
    write(path, bytes)?;
    helper(path, "save", None)
}

pub fn begin(path: &Path, identity: &str) -> io::Result<()> {
    require_clear(path)?;
    write(
        &pending_path(path),
        b"Unconfirmed delivery; reconcile before retrying\n",
    )?;
    helper(path, "begin", Some(identity))
}

pub fn confirm(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write(path, bytes)?;
    helper(path, "confirm", None)?;
    std::fs::remove_file(pending_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_send_and_failed_confirmation_block_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        save(&path, b"before").unwrap();
        begin(&path, "item-1").unwrap();
        assert!(begin(&path, "item-1").is_err());
        assert!(save(&path, b"before").is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(confirm(&path, b"acknowledged").is_err());
        assert!(begin(&path, "item-1").is_err());
        std::fs::remove_dir(&path).unwrap();
        confirm(&path, b"acknowledged").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"acknowledged");
        save(&path, b"acknowledged").unwrap();
    }
}
