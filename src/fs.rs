use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub trait FileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn is_file(&self, path: &Path) -> io::Result<bool>;
    fn is_dir(&self, path: &Path) -> io::Result<bool>;
    fn list_files(&self, root: &Path, recursive: bool) -> io::Result<Vec<PathBuf>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn write_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        fs::write(path, contents)
    }

    fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn is_file(&self, path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.is_file())
    }

    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.is_dir())
    }

    fn list_files(&self, root: &Path, recursive: bool) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let max_depth = if recursive { usize::MAX } else { 1 };

        for entry in WalkDir::new(root).max_depth(max_depth).min_depth(1) {
            let entry = entry?;
            if entry.file_type().is_file() {
                files.push(entry.into_path());
            }
        }

        files.sort();
        Ok(files)
    }
}
