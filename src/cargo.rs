use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompilerTrack {
    Llvm,
    Native,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoBuild {
    pub track: CompilerTrack,
    pub cargo: PathBuf,
    pub workspace_root: PathBuf,
    pub wrapper: PathBuf,
    pub rustc: Option<PathBuf>,
    pub target_dir: PathBuf,
    pub release: bool,
    pub offline: bool,
    pub locked: bool,
}

impl CargoBuild {
    pub fn new(
        track: CompilerTrack,
        workspace_root: impl Into<PathBuf>,
        wrapper: impl Into<PathBuf>,
    ) -> Self {
        let target_name = match track {
            CompilerTrack::Llvm => "kfrc-llvm",
            CompilerTrack::Native => "kfrc-native",
        };

        Self {
            track,
            cargo: PathBuf::from("cargo"),
            workspace_root: workspace_root.into(),
            wrapper: wrapper.into(),
            rustc: None,
            target_dir: Path::new("target").join(target_name),
            release: false,
            offline: false,
            locked: false,
        }
    }

    pub fn command(&self) -> Result<Command, CargoBuildError> {
        if self.track == CompilerTrack::Native {
            return Err(CargoBuildError::NativeCargoUnsupported);
        }

        validate_path("cargo executable", &self.cargo)?;
        validate_path("workspace root", &self.workspace_root)?;
        validate_path("rustc wrapper", &self.wrapper)?;
        validate_path("target directory", &self.target_dir)?;
        if let Some(rustc) = &self.rustc {
            validate_path("rustc executable", rustc)?;
            if rustc == &self.wrapper {
                return Err(CargoBuildError::RecursiveWrapper {
                    wrapper: self.wrapper.clone(),
                });
            }
        }

        let mut command = Command::new(&self.cargo);
        command
            .current_dir(&self.workspace_root)
            .arg("build")
            .arg("--target-dir")
            .arg(&self.target_dir)
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("RUSTC_WRAPPER", &self.wrapper)
            .env("RUSTC_WORKSPACE_WRAPPER", &self.wrapper);

        match &self.rustc {
            Some(rustc) => {
                command.env("RUSTC", rustc);
            }
            None => {
                command.env_remove("RUSTC");
            }
        }

        if self.release {
            command.arg("--release");
        }
        if self.offline {
            command.arg("--offline");
        }
        if self.locked {
            command.arg("--locked");
        }

        Ok(command)
    }
}

pub fn cargo_build_command(config: &CargoBuild) -> Result<Command, CargoBuildError> {
    config.command()
}

fn validate_path(field: &'static str, path: &Path) -> Result<(), CargoBuildError> {
    if path.as_os_str().is_empty() {
        Err(CargoBuildError::EmptyPath(field))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CargoBuildError {
    NativeCargoUnsupported,
    EmptyPath(&'static str),
    RecursiveWrapper { wrapper: PathBuf },
}

impl fmt::Display for CargoBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeCargoUnsupported => formatter.write_str(
                "the native track does not support full Cargo builds yet; use the LLVM reference track",
            ),
            Self::EmptyPath(field) => write!(formatter, "{field} must not be empty"),
            Self::RecursiveWrapper { wrapper } => write!(
                formatter,
                "rustc and the rustc wrapper must differ to avoid recursive wrapper invocation: {}",
                wrapper.display()
            ),
        }
    }
}

impl Error for CargoBuildError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn llvm_build() -> CargoBuild {
        CargoBuild::new(CompilerTrack::Llvm, "/workspace", "/tools/kfrc")
    }

    fn args(command: &Command) -> Vec<&OsStr> {
        command.get_args().collect()
    }

    fn environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(name))
            .map(|(_, value)| value)
    }

    #[test]
    fn constructs_llvm_build_with_all_options() {
        let mut config = llvm_build();
        config.release = true;
        config.offline = true;
        config.locked = true;

        let command = config.command().unwrap();

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(command.get_current_dir(), Some(Path::new("/workspace")));
        assert_eq!(
            args(&command),
            [
                OsStr::new("build"),
                OsStr::new("--target-dir"),
                OsStr::new("target/kfrc-llvm"),
                OsStr::new("--release"),
                OsStr::new("--offline"),
                OsStr::new("--locked"),
            ]
        );
        assert_eq!(
            environment(&command, "RUSTC_WRAPPER"),
            Some(Some(OsStr::new("/tools/kfrc")))
        );
        assert_eq!(
            environment(&command, "RUSTC_WORKSPACE_WRAPPER"),
            Some(Some(OsStr::new("/tools/kfrc")))
        );
        assert_eq!(
            environment(&command, "CARGO_TARGET_DIR"),
            Some(Some(OsStr::new("target/kfrc-llvm")))
        );
    }

    #[test]
    fn uses_an_isolated_target_directory_by_default() {
        let llvm = llvm_build();
        let native = CargoBuild::new(CompilerTrack::Native, "/workspace", "/tools/kfrc");

        assert_eq!(llvm.target_dir, Path::new("target/kfrc-llvm"));
        assert_eq!(native.target_dir, Path::new("target/kfrc-native"));
        assert_eq!(
            args(&llvm.command().unwrap()),
            [
                OsStr::new("build"),
                OsStr::new("--target-dir"),
                OsStr::new("target/kfrc-llvm"),
            ]
        );
    }

    #[test]
    fn removes_inherited_rustc_to_prevent_recursive_wrappers() {
        let command = llvm_build().command().unwrap();

        assert_eq!(environment(&command, "RUSTC"), Some(None));
    }

    #[test]
    fn forwards_an_explicit_non_recursive_rustc() {
        let mut config = llvm_build();
        config.rustc = Some(PathBuf::from("/toolchain/bin/rustc"));

        let command = config.command().unwrap();

        assert_eq!(
            environment(&command, "RUSTC"),
            Some(Some(OsStr::new("/toolchain/bin/rustc")))
        );
    }

    #[test]
    fn rejects_rustc_that_is_the_wrapper() {
        let mut config = llvm_build();
        config.rustc = Some(config.wrapper.clone());

        assert_eq!(
            config.command().unwrap_err(),
            CargoBuildError::RecursiveWrapper {
                wrapper: PathBuf::from("/tools/kfrc"),
            }
        );
    }

    #[test]
    fn rejects_native_full_cargo_builds_explicitly() {
        let config = CargoBuild::new(CompilerTrack::Native, "/workspace", "/tools/kfrc");

        let error = config.command().unwrap_err();

        assert_eq!(error, CargoBuildError::NativeCargoUnsupported);
        assert!(
            error
                .to_string()
                .contains("does not support full Cargo builds yet")
        );
        assert!(error.to_string().contains("LLVM reference track"));
    }

    #[test]
    fn honors_a_custom_target_directory() {
        let mut config = llvm_build();
        config.target_dir = PathBuf::from("/artifacts/kfrc-reference");

        let command = config.command().unwrap();

        assert_eq!(
            args(&command),
            [
                OsStr::new("build"),
                OsStr::new("--target-dir"),
                OsStr::new("/artifacts/kfrc-reference"),
            ]
        );
        assert_eq!(
            environment(&command, "CARGO_TARGET_DIR"),
            Some(Some(OsStr::new("/artifacts/kfrc-reference")))
        );
    }

    #[test]
    fn rejects_empty_required_paths() {
        let mut config = llvm_build();
        config.wrapper = PathBuf::new();

        assert_eq!(
            config.command().unwrap_err(),
            CargoBuildError::EmptyPath("rustc wrapper")
        );
    }
}
