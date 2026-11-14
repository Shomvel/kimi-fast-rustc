//! Reproducible rustc/LLVM options for the reference compiler track.
//!
//! This module is independent of the native backend and never enables unsafe
//! floating-point optimizations.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptLevel {
    Speed,
    Balanced,
    Size,
}

impl OptLevel {
    fn rustc_value(self) -> &'static str {
        match self {
            Self::Speed => "3",
            Self::Balanced => "2",
            Self::Size => "z",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lto {
    Thin,
    Fat,
}

impl Lto {
    fn rustc_value(self) -> &'static str {
        match self {
            Self::Thin => "thin",
            Self::Fat => "fat",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetCpu {
    Generic,
    Native,
}

impl TargetCpu {
    fn rustc_value(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Native => "native",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LlvmConfig {
    pub opt_level: OptLevel,
    pub lto: Lto,
    pub codegen_units: usize,
    pub panic_abort: bool,
    pub strip: bool,
    pub target_cpu: TargetCpu,
}

impl Default for LlvmConfig {
    fn default() -> Self {
        Self {
            opt_level: OptLevel::Speed,
            lto: Lto::Thin,
            codegen_units: 1,
            panic_abort: true,
            strip: true,
            target_cpu: TargetCpu::Generic,
        }
    }
}

impl LlvmConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.codegen_units == 0 {
            return Err(ConfigError::InvalidCodegenUnits(0));
        }
        Ok(())
    }

    pub fn rustc_args(&self) -> Result<Vec<String>, ConfigError> {
        self.validate()?;

        let panic = if self.panic_abort { "abort" } else { "unwind" };
        let strip = if self.strip { "symbols" } else { "none" };
        let options = [
            format!("opt-level={}", self.opt_level.rustc_value()),
            format!("lto={}", self.lto.rustc_value()),
            format!("codegen-units={}", self.codegen_units),
            format!("panic={panic}"),
            format!("strip={strip}"),
            format!("target-cpu={}", self.target_cpu.rustc_value()),
            "embed-bitcode=yes".to_owned(),
        ];
        let mut args = Vec::with_capacity(options.len() * 2);
        for option in options {
            args.push("-C".to_owned());
            args.push(option);
        }
        Ok(args)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidCodegenUnits(usize),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCodegenUnits(value) => {
                write!(
                    formatter,
                    "codegen-units must be greater than zero (got {value})"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn default_args_are_explicit_and_reproducible() {
        let config = LlvmConfig::default();
        let expected = owned(&[
            "-C",
            "opt-level=3",
            "-C",
            "lto=thin",
            "-C",
            "codegen-units=1",
            "-C",
            "panic=abort",
            "-C",
            "strip=symbols",
            "-C",
            "target-cpu=generic",
            "-C",
            "embed-bitcode=yes",
        ]);

        assert_eq!(config.rustc_args().unwrap(), expected);
        assert_eq!(config.rustc_args().unwrap(), config.rustc_args().unwrap());
    }

    #[test]
    fn optimization_levels_map_to_stable_rustc_values() {
        for (opt_level, value) in [
            (OptLevel::Speed, "opt-level=3"),
            (OptLevel::Balanced, "opt-level=2"),
            (OptLevel::Size, "opt-level=z"),
        ] {
            let config = LlvmConfig {
                opt_level,
                ..LlvmConfig::default()
            };
            assert_eq!(config.rustc_args().unwrap()[1], value);
        }
    }

    #[test]
    fn explicit_overrides_cover_all_supported_options() {
        let config = LlvmConfig {
            opt_level: OptLevel::Speed,
            lto: Lto::Fat,
            codegen_units: 4,
            panic_abort: false,
            strip: false,
            target_cpu: TargetCpu::Native,
        };

        assert_eq!(
            config.rustc_args().unwrap(),
            owned(&[
                "-C",
                "opt-level=3",
                "-C",
                "lto=fat",
                "-C",
                "codegen-units=4",
                "-C",
                "panic=unwind",
                "-C",
                "strip=none",
                "-C",
                "target-cpu=native",
                "-C",
                "embed-bitcode=yes",
            ])
        );
    }

    #[test]
    fn fat_lto_does_not_reject_valid_multiple_codegen_units() {
        let config = LlvmConfig {
            lto: Lto::Fat,
            codegen_units: 2,
            ..LlvmConfig::default()
        };

        assert!(config.validate().is_ok());
        assert!(config.rustc_args().is_ok());
    }

    #[test]
    fn zero_codegen_units_is_rejected() {
        let config = LlvmConfig {
            codegen_units: 0,
            ..LlvmConfig::default()
        };
        assert_eq!(config.validate(), Err(ConfigError::InvalidCodegenUnits(0)));
        assert_eq!(
            config.rustc_args(),
            Err(ConfigError::InvalidCodegenUnits(0))
        );
        assert_eq!(
            ConfigError::InvalidCodegenUnits(0).to_string(),
            "codegen-units must be greater than zero (got 0)"
        );
    }

    #[test]
    fn no_configuration_enables_unsafe_fast_math() {
        let forbidden = ["fast-math", "unsafe-fp", "reassoc", "nnan", "ninf"];

        for opt_level in [OptLevel::Speed, OptLevel::Balanced, OptLevel::Size] {
            for lto in [Lto::Thin, Lto::Fat] {
                for target_cpu in [TargetCpu::Generic, TargetCpu::Native] {
                    let config = LlvmConfig {
                        opt_level,
                        lto,
                        target_cpu,
                        ..LlvmConfig::default()
                    };
                    let args = config.rustc_args().unwrap();
                    assert!(
                        args.iter()
                            .all(|arg| { forbidden.iter().all(|flag| !arg.contains(flag)) })
                    );
                }
            }
        }
    }
}
