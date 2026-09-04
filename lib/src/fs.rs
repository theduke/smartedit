use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use walkdir::WalkDir;

/// Filesystem operations used during planning and execution.
///
/// Implementations must make `create_new_bytes` and non-overwriting `move_file` fail atomically
/// with [`io::ErrorKind::AlreadyExists`] when the destination directory entry already exists,
/// including when that entry is a dangling symbolic link.
pub trait FileSystem {
    /// Whether this implementation supplies enforceable entry/content identity preconditions.
    fn identity_checks_supported(&self) -> bool {
        false
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    /// Creates and writes a new file without replacing an existing directory entry.
    fn create_new_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Moves one file entry, replacing the destination only when `overwrite` is true.
    fn move_file(&self, source: &Path, destination: &Path, overwrite: bool) -> io::Result<()>;
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn is_file(&self, path: &Path) -> io::Result<bool>;
    fn is_dir(&self, path: &Path) -> io::Result<bool>;
    fn is_symlink(&self, _path: &Path) -> io::Result<bool> {
        Ok(false)
    }
    fn list_files(&self, root: &Path, recursive: bool) -> io::Result<Vec<PathBuf>>;

    /// Reports whether two paths currently resolve to the same file contents.
    fn same_content_file(&self, left: &Path, right: &Path) -> io::Result<bool> {
        Ok(matches!(
            (self.content_identity(left)?, self.content_identity(right)?),
            (Some(left), Some(right)) if left == right
        ))
    }

    /// Returns the identity of the content reached by `path`, following a final symbolic link.
    ///
    /// The default reports that identity checks are unsupported. Implementations that return an
    /// identity should also override `write_bytes_checked` so the check and opening the file are
    /// performed against the same object.
    fn content_identity(&self, _path: &Path) -> io::Result<Option<FileIdentity>> {
        Ok(None)
    }

    /// Returns the identity of the directory entry at `path`, without following a final symlink.
    fn entry_identity(&self, _path: &Path) -> io::Result<Option<FileIdentity>> {
        Ok(None)
    }

    /// Writes only if the current content identity is still `expected`.
    fn write_bytes_checked(
        &self,
        path: &Path,
        contents: &[u8],
        expected: &FileIdentity,
    ) -> io::Result<()> {
        if self.content_identity(path)?.as_ref() != Some(expected) {
            return Err(identity_mismatch("file changed before write"));
        }
        self.write_bytes(path, contents)
    }

    /// Removes only if the current entry identity is still `expected`.
    fn remove_file_checked(&self, path: &Path, expected: &FileIdentity) -> io::Result<()> {
        if self.entry_identity(path)?.as_ref() != Some(expected) {
            return Err(identity_mismatch("file changed before delete"));
        }
        self.remove_file(path)
    }
}

/// An opaque filesystem identity captured while evaluating a plan.
#[derive(Debug, Clone)]
pub struct FileIdentity(EntryIdentity);

impl FileIdentity {
    /// Creates an identity token for a custom [`FileSystem`] implementation.
    ///
    /// Tokens only compare equal to custom tokens containing the same bytes; they never compare
    /// equal to identities produced by [`OsFileSystem`].
    pub fn custom(token: impl Into<Vec<u8>>) -> Self {
        Self(EntryIdentity::Custom(token.into().into()))
    }
}

impl PartialEq for FileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.matches_identity(&other.0)
    }
}

impl Eq for FileIdentity {}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    fn identity_checks_supported(&self) -> bool {
        true
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn write_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        fs::write(path, contents)
    }

    fn write_bytes_checked(
        &self,
        path: &Path,
        contents: &[u8],
        expected: &FileIdentity,
    ) -> io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).open(path)?;
        let actual = EntryIdentity::from_file(&file)?;
        verify_expected_identity(&actual, Some(&expected.0))?;
        file.set_len(0)?;
        file.write_all(contents)
    }

    fn create_new_bytes(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(contents)
    }

    fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn remove_file_checked(&self, path: &Path, expected: &FileIdentity) -> io::Result<()> {
        remove_entry_if_matches(path, &expected.0, "file changed before delete")
    }

    fn move_file(&self, source: &Path, destination: &Path, overwrite: bool) -> io::Result<()> {
        if !overwrite {
            return move_file_no_replace(source, destination);
        }
        if !entry_exists(destination)? {
            return move_file_no_replace(source, destination);
        }

        let (backup, backup_identity) = move_entry_to_unused_backup(destination)?;
        match move_file_no_replace(source, destination) {
            Ok(()) => remove_entry_if_matches(
                &backup,
                &backup_identity,
                "backup entry changed before cleanup; backup retained",
            ),
            Err(move_error) => {
                match move_file_no_replace_expected(&backup, destination, &backup_identity) {
                    Ok(()) => Err(move_error),
                    Err(restore_error) => {
                        // Do not remove or replace an entry that appeared during the failed move.
                        // The backup pathname is left untouched; if its identity changed, the
                        // original backup may have been moved elsewhere by another process.
                        Err(io::Error::other(format!(
                            "move failed: {move_error}; restoring the original destination failed: {restore_error}; backup path left untouched at {}",
                            backup.display()
                        )))
                    }
                }
            }
        }
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        entry_exists(path)
    }

    fn is_file(&self, path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.is_file())
    }

    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.is_dir())
    }

    fn is_symlink(&self, path: &Path) -> io::Result<bool> {
        Ok(fs::symlink_metadata(path)?.file_type().is_symlink())
    }

    fn list_files(&self, root: &Path, recursive: bool) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let max_depth = if recursive { usize::MAX } else { 1 };

        for entry in WalkDir::new(root).max_depth(max_depth).min_depth(1) {
            let entry = entry?;
            if entry.file_type().is_file() || entry.file_type().is_symlink() {
                files.push(entry.into_path());
            }
        }

        files.sort();
        Ok(files)
    }

    fn content_identity(&self, path: &Path) -> io::Result<Option<FileIdentity>> {
        let file = fs::File::open(path)?;
        Ok(Some(FileIdentity(EntryIdentity::from_file(&file)?)))
    }

    fn entry_identity(&self, path: &Path) -> io::Result<Option<FileIdentity>> {
        Ok(Some(FileIdentity(EntryIdentity::from_path(path)?)))
    }

    fn same_content_file(&self, left: &Path, right: &Path) -> io::Result<bool> {
        same_file::is_same_file(left, right)
    }
}

fn identity_mismatch(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}

fn move_file_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    move_file_no_replace_retained(source, destination, None).map(drop)
}

fn move_file_no_replace_expected(
    source: &Path,
    destination: &Path,
    expected_source: &EntryIdentity,
) -> io::Result<()> {
    move_file_no_replace_retained(source, destination, Some(expected_source)).map(drop)
}

fn move_file_no_replace_retained(
    source: &Path,
    destination: &Path,
    expected_source: Option<&EntryIdentity>,
) -> io::Result<EntryIdentity> {
    let file_type = fs::symlink_metadata(source)?.file_type();
    if file_type.is_symlink() {
        let source_identity = EntryIdentity::from_path(source)?;
        verify_expected_identity(&source_identity, expected_source)?;
        create_symlink_no_replace(source, destination)?;
        let destination_identity = EntryIdentity::from_path(destination)?;
        if !source_identity.matches_path(source)? {
            let _ = remove_entry_if_matches(
                destination,
                &destination_identity,
                "published symbolic link changed before cleanup",
            );
            return Err(io::Error::other(
                "source entry changed while publishing; source retained",
            ));
        }
        remove_entry_if_matches(
            source,
            &source_identity,
            "source entry changed before cleanup; source retained",
        )?;
        return Ok(destination_identity);
    }

    let mut source_file = fs::File::open(source)?;
    let source_identity = EntryIdentity::from_file(&source_file)?;
    verify_expected_identity(&source_identity, expected_source)?;
    if !source_identity.matches_path(source)? {
        return Err(io::Error::other(
            "source entry changed before publication; source retained",
        ));
    }

    match fs::hard_link(source, destination) {
        Ok(()) => {
            if !source_identity.matches_path(destination)? {
                if let Ok(destination_identity) = EntryIdentity::from_path(destination) {
                    let _ = remove_entry_if_matches(
                        destination,
                        &destination_identity,
                        "published destination changed before cleanup",
                    );
                }
                return Err(io::Error::other(
                    "destination does not reference the retained source; source retained",
                ));
            }
            remove_entry_if_matches(
                source,
                &source_identity,
                "source entry changed before cleanup; source retained",
            )?;
            Ok(source_identity)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        Err(link_error) => {
            if !file_type.is_file() {
                return Err(link_error);
            }

            let permissions = source_file.metadata()?.permissions();
            let destination_identity =
                copy_open_file_no_replace(&mut source_file, permissions, destination)?;
            remove_entry_if_matches(
                source,
                &source_identity,
                "source entry changed before cleanup; source retained",
            )?;
            Ok(destination_identity)
        }
    }
}

#[cfg(test)]
fn copy_file_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let mut source_file = fs::File::open(source)?;
    let permissions = source_file.metadata()?.permissions();
    copy_open_file_no_replace(&mut source_file, permissions, destination).map(drop)
}

fn copy_open_file_no_replace(
    source_file: &mut fs::File,
    permissions: fs::Permissions,
    destination: &Path,
) -> io::Result<EntryIdentity> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        copy_file_via_anonymous_staging(source_file, permissions, destination)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        copy_file_via_exclusive_destination(source_file, permissions, destination)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn copy_file_via_anonymous_staging(
    source_file: &mut fs::File,
    permissions: fs::Permissions,
    destination: &Path,
) -> io::Result<EntryIdentity> {
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let parent = CString::new(parent.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination parent contains a NUL byte",
        )
    })?;
    // SAFETY: `parent` is a valid NUL-terminated path and the returned descriptor is checked
    // before being transferred into `File` exactly once.
    let descriptor = unsafe {
        libc::open(
            parent.as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor == -1 {
        // Some network and older filesystems do not implement `O_TMPFILE`. The fallback still
        // creates the final name exclusively and verifies its identity before source removal.
        return copy_file_via_exclusive_destination(source_file, permissions, destination);
    }
    // SAFETY: ownership of the successful `open` descriptor is transferred to this `File`.
    let mut staging_file = unsafe { fs::File::from_raw_fd(descriptor) };
    io::copy(source_file, &mut staging_file)?;
    staging_file.flush()?;
    // Writing can clear setuid/setgid bits on Unix, so restore the source mode only after the
    // copied contents are complete.
    staging_file.set_permissions(permissions)?;

    let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination contains a NUL byte",
        )
    })?;
    let empty_path = c"";
    // SAFETY: both C strings are valid for the duration of the call. `staging_file` remains open,
    // so publication is bound to the copied inode even if directory entries are concurrently
    // changed. `linkat` fails with `EEXIST` instead of replacing `destination`.
    let result = unsafe {
        libc::linkat(
            staging_file.as_raw_fd(),
            CStr::as_ptr(empty_path),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    let identity = EntryIdentity::from_file(&staging_file)?;
    if !identity.matches_path(destination)? {
        let _ = remove_entry_if_matches(
            destination,
            &identity,
            "published destination changed before cleanup",
        );
        return Err(io::Error::other(
            "published destination does not reference the staged file",
        ));
    }
    Ok(identity)
}

#[cfg(unix)]
fn create_symlink_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

#[cfg(windows)]
fn create_symlink_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    let target = fs::read_link(source)?;
    let file_type = fs::symlink_metadata(source)?.file_type();
    if file_type.is_symlink_dir() {
        std::os::windows::fs::symlink_dir(target, destination)
    } else if file_type.is_symlink_file() {
        std::os::windows::fs::symlink_file(target, destination)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a symbolic link", source.display()),
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn create_symlink_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "moving symbolic links is unsupported on this platform",
    ))
}

fn entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn copy_file_via_exclusive_destination(
    source_file: &mut fs::File,
    permissions: fs::Permissions,
    destination: &Path,
) -> io::Result<EntryIdentity> {
    let mut destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let identity = EntryIdentity::from_file(&destination_file)?;
    let copy_result = (|| {
        io::copy(source_file, &mut destination_file)?;
        destination_file.flush()?;
        // Writing can clear setuid/setgid bits on Unix, so restore the source mode last.
        destination_file.set_permissions(permissions)
    })();
    if let Err(error) = copy_result {
        let _ = remove_entry_if_matches(
            destination,
            &identity,
            "partial destination changed before cleanup",
        );
        return Err(error);
    }
    if !identity.matches_path(destination)? {
        let _ = remove_entry_if_matches(
            destination,
            &identity,
            "copied destination changed before cleanup",
        );
        return Err(io::Error::other(
            "destination entry changed while copying; source retained",
        ));
    }
    Ok(identity)
}

#[derive(Debug, Clone)]
enum EntryIdentity {
    File(Arc<same_file::Handle>),
    Custom(Arc<[u8]>),
    #[cfg(unix)]
    Symlink {
        device: u64,
        inode: u64,
    },
    #[cfg(windows)]
    Symlink(Arc<same_file::Handle>),
}

impl EntryIdentity {
    fn from_file(file: &fs::File) -> io::Result<Self> {
        Ok(Self::File(Arc::new(same_file::Handle::from_file(
            file.try_clone()?,
        )?)))
    }

    fn from_path(path: &Path) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_symlink() {
            return Ok(Self::File(Arc::new(same_file::Handle::from_path(path)?)));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            Ok(Self::Symlink {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            // Opening the reparse point itself, rather than following it, also works for a
            // dangling symbolic link. `FILE_FLAG_BACKUP_SEMANTICS` permits directory symlinks.
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)?;
            Ok(Self::Symlink(Arc::new(same_file::Handle::from_file(file)?)))
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = metadata;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "retaining symbolic-link identity is unsupported on this platform",
            ))
        }
    }

    fn matches_path(&self, path: &Path) -> io::Result<bool> {
        let current = Self::from_path(path)?;
        Ok(self.matches_identity(&current))
    }

    fn matches_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(left), Self::File(right)) => left == right,
            (Self::Custom(left), Self::Custom(right)) => left == right,
            #[cfg(unix)]
            (
                Self::Symlink {
                    device: left_device,
                    inode: left_inode,
                },
                Self::Symlink {
                    device: right_device,
                    inode: right_inode,
                },
            ) => left_device == right_device && left_inode == right_inode,
            #[cfg(windows)]
            (Self::Symlink(left), Self::Symlink(right)) => left == right,
            _ => false,
        }
    }
}

fn verify_expected_identity(
    actual: &EntryIdentity,
    expected: Option<&EntryIdentity>,
) -> io::Result<()> {
    if expected.is_some_and(|expected| !expected.matches_identity(actual)) {
        return Err(io::Error::other(
            "source entry changed before the move; entry retained",
        ));
    }
    Ok(())
}

fn remove_entry_if_matches(
    path: &Path,
    identity: &EntryIdentity,
    mismatch_message: &str,
) -> io::Result<()> {
    match identity.matches_path(path) {
        Ok(true) => fs::remove_file(path),
        Ok(false) => Err(io::Error::other(mismatch_message.to_owned())),
        Err(error) => Err(io::Error::other(format!(
            "{mismatch_message}: could not verify {}: {error}",
            path.display()
        ))),
    }
}

fn move_entry_to_unused_backup(destination: &Path) -> io::Result<(PathBuf, EntryIdentity)> {
    static NEXT_BACKUP: AtomicU64 = AtomicU64::new(0);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("destination");

    for _ in 0..100 {
        let suffix = NEXT_BACKUP.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{name}.smartedit-backup-{}-{suffix}",
            std::process::id()
        ));
        match move_file_no_replace_retained(destination, &candidate, None) {
            Ok(identity) => return Ok((candidate, identity)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "could not allocate a backup path beside {}",
            destination.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        EntryIdentity, FileSystem, OsFileSystem, copy_file_no_replace,
        move_file_no_replace_expected, remove_entry_if_matches,
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("smartedit-fs-{name}-{}-{unique}", process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_replace_move_does_not_clobber_an_existing_file() {
        let dir = TestDir::new("no-clobber");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "source").unwrap();
        fs::write(&destination, "destination").unwrap();

        let error = OsFileSystem
            .move_file(&source, &destination, false)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(source).unwrap(), "source");
        assert_eq!(fs::read_to_string(destination).unwrap(), "destination");
    }

    #[test]
    fn staged_copy_publishes_only_complete_content() {
        let dir = TestDir::new("staged-copy");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "complete content").unwrap();

        copy_file_no_replace(&source, &destination).unwrap();

        assert_eq!(fs::read_to_string(&source).unwrap(), "complete content");
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "complete content"
        );
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("smartedit-staging")
        }));
    }

    #[test]
    fn cleanup_rejects_a_replaced_source_entry() {
        let dir = TestDir::new("source-replacement");
        let source = dir.path().join("source.txt");
        let moved_original = dir.path().join("original.txt");
        fs::write(&source, "original").unwrap();
        let identity = EntryIdentity::from_path(&source).unwrap();

        fs::rename(&source, &moved_original).unwrap();
        fs::write(&source, "replacement").unwrap();

        let error = remove_entry_if_matches(
            &source,
            &identity,
            "source entry changed before cleanup; source retained",
        )
        .unwrap_err();

        assert!(error.to_string().contains("source entry changed"));
        assert_eq!(fs::read_to_string(source).unwrap(), "replacement");
        assert_eq!(fs::read_to_string(moved_original).unwrap(), "original");
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_compares_symlink_entries_without_following_them() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("symlink-source-replacement");
        let source = dir.path().join("source-link");
        let moved_original = dir.path().join("original-link");
        symlink("missing-original-target", &source).unwrap();
        let identity = EntryIdentity::from_path(&source).unwrap();

        fs::rename(&source, &moved_original).unwrap();
        symlink("missing-replacement-target", &source).unwrap();

        remove_entry_if_matches(
            &source,
            &identity,
            "source entry changed before cleanup; source retained",
        )
        .unwrap_err();

        assert_eq!(
            fs::read_link(source).unwrap(),
            Path::new("missing-replacement-target")
        );
        assert_eq!(
            fs::read_link(moved_original).unwrap(),
            Path::new("missing-original-target")
        );
    }

    #[test]
    fn cleanup_rejects_a_replaced_backup_entry() {
        let dir = TestDir::new("backup-replacement");
        let backup = dir.path().join("backup.txt");
        let moved_original = dir.path().join("original-backup.txt");
        fs::write(&backup, "original destination").unwrap();
        let identity = EntryIdentity::from_path(&backup).unwrap();

        fs::rename(&backup, &moved_original).unwrap();
        fs::write(&backup, "replacement").unwrap();

        let error = remove_entry_if_matches(
            &backup,
            &identity,
            "backup entry changed before cleanup; backup retained",
        )
        .unwrap_err();

        assert!(error.to_string().contains("backup entry changed"));
        assert_eq!(fs::read_to_string(backup).unwrap(), "replacement");
        assert_eq!(
            fs::read_to_string(moved_original).unwrap(),
            "original destination"
        );
    }

    #[test]
    fn restore_rejects_a_replaced_backup_entry() {
        let dir = TestDir::new("backup-restore-replacement");
        let backup = dir.path().join("backup.txt");
        let moved_original = dir.path().join("original-backup.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&backup, "original destination").unwrap();
        let identity = EntryIdentity::from_path(&backup).unwrap();

        fs::rename(&backup, &moved_original).unwrap();
        fs::write(&backup, "replacement").unwrap();

        let error = move_file_no_replace_expected(&backup, &destination, &identity).unwrap_err();

        assert!(error.to_string().contains("source entry changed"));
        assert!(!destination.exists());
        assert_eq!(fs::read_to_string(backup).unwrap(), "replacement");
        assert_eq!(
            fs::read_to_string(moved_original).unwrap(),
            "original destination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn staged_copy_preserves_unix_special_mode_bits() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = TestDir::new("staged-copy-mode");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "content").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o6751)).unwrap();

        copy_file_no_replace(&source, &destination).unwrap();

        assert_eq!(fs::metadata(destination).unwrap().mode() & 0o7777, 0o6751);
    }

    #[test]
    fn overwrite_move_restores_the_destination_when_the_source_move_fails() {
        let dir = TestDir::new("overwrite-restore");
        let missing_source = dir.path().join("missing.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&destination, "original").unwrap();

        OsFileSystem
            .move_file(&missing_source, &destination, true)
            .unwrap_err();

        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("smartedit-backup")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn no_replace_move_treats_a_dangling_destination_symlink_as_occupied() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("dangling-destination");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "source").unwrap();
        symlink(dir.path().join("missing-target"), &destination).unwrap();

        let error = OsFileSystem
            .move_file(&source, &destination, false)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(source).unwrap(), "source");
        assert!(
            fs::symlink_metadata(destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn portable_copy_removes_its_own_partial_destination_after_failure() {
        let dir = TestDir::new("portable-partial-cleanup");
        let source_directory = dir.path().join("source-directory");
        let destination = dir.path().join("destination.txt");
        fs::create_dir(&source_directory).unwrap();
        let mut source = fs::File::open(&source_directory).unwrap();
        let permissions = source.metadata().unwrap().permissions();

        super::copy_file_via_exclusive_destination(&mut source, permissions, &destination)
            .unwrap_err();

        assert!(fs::symlink_metadata(destination).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_listing_includes_symlink_entries_without_following_child_directories() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new("list-symlinks");
        let child_directory = dir.path().join("child-directory");
        let directory_link = dir.path().join("directory-link");
        let dangling_link = dir.path().join("dangling-link");
        fs::create_dir(&child_directory).unwrap();
        fs::write(child_directory.join("nested.txt"), "nested").unwrap();
        symlink(&child_directory, &directory_link).unwrap();
        symlink("missing", &dangling_link).unwrap();

        let files = OsFileSystem.list_files(dir.path(), true).unwrap();

        assert!(files.contains(&directory_link));
        assert!(files.contains(&dangling_link));
        assert!(!files.contains(&directory_link.join("nested.txt")));
    }
}
