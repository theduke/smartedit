use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{ArgAction, ArgGroup, Args};

use crate::cli_support::{resolve_root, write_stdout};

const SKILL_NAME: &str = "smartedit";
const SKILL_CONTENT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/skill/SKILL.md"));

#[derive(Debug, Args)]
#[command(
    about = "Install the bundled smartedit agent skill",
    long_about = "Install the bundled smartedit agent skill below a repository, a directory, or your user home.\n\nThe skill is written to .agents/skills/smartedit/SKILL.md below the selected root. The installed skill invokes `smartedit`, so the executable must remain available on PATH.",
    after_help = "Examples:\n  smartedit install-skill --repo\n  smartedit install-skill --repo path/to/checkout\n  smartedit install-skill --dir path/to/project\n  smartedit install-skill --user"
)]
#[command(group(
    ArgGroup::new("target")
        .required(true)
        .args(["repo", "user", "dir"])
))]
pub struct CmdInstallSkill {
    /// Find the nearest Git repository from PATH, or from the current directory.
    #[arg(long, action = ArgAction::SetTrue)]
    pub repo: bool,

    /// Install below the current user's cross-platform home directory.
    #[arg(long, action = ArgAction::SetTrue)]
    pub user: bool,

    /// Install below PATH, or below the current directory when PATH is omitted.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dir: bool,

    /// Starting path for --repo or installation root for --dir.
    #[arg(
        value_name = "PATH",
        help = "Starting path for --repo or root for --dir [default: current directory]"
    )]
    pub path: Option<PathBuf>,
}

impl CmdInstallSkill {
    pub fn run(&self) -> Result<(), String> {
        let current_dir =
            env::current_dir().map_err(|error| format!("failed to get cwd: {error}"))?;
        if self.user && self.path.is_some() {
            return Err("`--user` does not accept a path argument".to_owned());
        }

        let install_root = if self.user {
            resolve_user_root()?
        } else {
            let start = resolve_root(self.path.as_deref(), &current_dir);
            if self.repo {
                find_repo_root(&start)?
            } else {
                normalize_start_dir(&start)?
            }
        };

        let skill_dir = install_skill(&install_root)?;
        write_stdout(&format!(
            "Installed `{SKILL_NAME}` skill to {}\n",
            skill_dir.display()
        ))
    }
}

fn install_skill(root: &Path) -> Result<PathBuf, String> {
    let skill_dir = root.join(".agents").join("skills").join(SKILL_NAME);
    fs::create_dir_all(&skill_dir)
        .map_err(|error| format!("failed to create {}: {error}", skill_dir.display()))?;

    let skill_file = skill_dir.join("SKILL.md");
    fs::write(&skill_file, SKILL_CONTENT)
        .map_err(|error| format!("failed to write {}: {error}", skill_file.display()))?;

    Ok(skill_dir)
}

fn resolve_user_root() -> Result<PathBuf, String> {
    resolve_user_root_from(|name| env::var_os(name), cfg!(windows))
}

fn resolve_user_root_from(
    mut get_var: impl FnMut(&str) -> Option<OsString>,
    is_windows: bool,
) -> Result<PathBuf, String> {
    if is_windows {
        if let Some(profile) = nonempty_env_value(get_var("USERPROFILE")) {
            return Ok(PathBuf::from(profile));
        }
        if let (Some(drive), Some(home_path)) = (
            nonempty_env_value(get_var("HOMEDRIVE")),
            nonempty_env_value(get_var("HOMEPATH")),
        ) {
            let mut combined = drive;
            combined.push(home_path);
            return Ok(PathBuf::from(combined));
        }
    }

    nonempty_env_value(get_var("HOME"))
        .or_else(|| nonempty_env_value(get_var("USERPROFILE")))
        .map(PathBuf::from)
        .ok_or_else(|| {
            "failed to resolve the user home from HOME, USERPROFILE, or HOMEDRIVE and HOMEPATH"
                .to_owned()
        })
}

fn nonempty_env_value(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

fn find_repo_root(start: &Path) -> Result<PathBuf, String> {
    let start_dir = normalize_start_dir(start)?;
    for ancestor in start_dir.ancestors() {
        let git_dir = ancestor.join(".git");
        if git_dir
            .try_exists()
            .map_err(|error| format!("failed to inspect {}: {error}", git_dir.display()))?
        {
            return Ok(ancestor.to_path_buf());
        }
    }

    Err(format!(
        "failed to find a repository root from {}",
        start.display()
    ))
}

fn normalize_start_dir(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.is_dir() {
        Ok(path.to_path_buf())
    } else if metadata.is_file() {
        path.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("file path has no parent directory: {}", path.display()))
    } else {
        Err(format!(
            "path is neither a regular file nor directory: {}",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        SKILL_CONTENT, find_repo_root, install_skill, normalize_start_dir, resolve_user_root_from,
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "smartedit-install-skill-{name}-{}-{unique}",
                process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn repo_root_is_found_from_nested_directory() {
        let dir = TestDir::new("repo-root-dir");
        let nested = dir.path().join("repo/src/bin");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(dir.path().join("repo/.git")).unwrap();

        let resolved = find_repo_root(&nested).unwrap();

        assert_eq!(resolved, dir.path().join("repo"));
    }

    #[test]
    fn repo_root_is_found_from_file_path() {
        let dir = TestDir::new("repo-root-file");
        let src_dir = dir.path().join("repo/src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(
            dir.path().join("repo/.git"),
            "gitdir: .git/worktrees/test\n",
        )
        .unwrap();

        let resolved = find_repo_root(&src_dir.join("main.rs")).unwrap();

        assert_eq!(resolved, dir.path().join("repo"));
    }

    #[test]
    fn normalize_start_dir_uses_parent_for_files() {
        let dir = TestDir::new("normalize-file");
        let file = dir.path().join("nested/input.txt");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "content\n").unwrap();

        let resolved = normalize_start_dir(&file).unwrap();

        assert_eq!(resolved, dir.path().join("nested"));
    }

    #[test]
    fn install_skill_creates_expected_layout_and_content() {
        let dir = TestDir::new("install");

        let installed = install_skill(dir.path()).unwrap();

        let skill_file = dir.path().join(".agents/skills/smartedit/SKILL.md");
        assert_eq!(installed, dir.path().join(".agents/skills/smartedit"));
        assert_eq!(fs::read_to_string(skill_file).unwrap(), SKILL_CONTENT);
    }

    #[test]
    fn user_root_uses_platform_precedence_and_ignores_empty_values() {
        let variables = HashMap::from([
            ("HOME", OsString::from("/home/tester")),
            ("USERPROFILE", OsString::from(r"C:\Users\tester")),
        ]);

        assert_eq!(
            resolve_user_root_from(|name| variables.get(name).cloned(), false).unwrap(),
            PathBuf::from("/home/tester")
        );
        assert_eq!(
            resolve_user_root_from(|name| variables.get(name).cloned(), true).unwrap(),
            PathBuf::from(r"C:\Users\tester")
        );

        let variables = HashMap::from([
            ("HOME", OsString::new()),
            ("USERPROFILE", OsString::from(r"C:\Users\tester")),
        ]);
        assert_eq!(
            resolve_user_root_from(|name| variables.get(name).cloned(), false).unwrap(),
            PathBuf::from(r"C:\Users\tester")
        );
    }

    #[test]
    fn user_root_falls_back_to_windows_home_parts() {
        let variables = HashMap::from([
            ("HOMEDRIVE", OsString::from("C:")),
            ("HOMEPATH", OsString::from(r"\Users\tester")),
        ]);

        assert_eq!(
            resolve_user_root_from(|name| variables.get(name).cloned(), true).unwrap(),
            PathBuf::from(r"C:\Users\tester")
        );
    }

    #[test]
    fn user_root_error_names_supported_environment_variables() {
        let error = resolve_user_root_from(|_| None, true).unwrap_err();

        assert!(error.contains("HOME"));
        assert!(error.contains("USERPROFILE"));
        assert!(error.contains("HOMEDRIVE"));
        assert!(error.contains("HOMEPATH"));
    }
}
