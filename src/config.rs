use std::env;
use std::fmt;
use std::str::FromStr;

pub const ENV_BACKEND: &str = "KFRC_BACKEND";
pub const ENV_OPT_LEVEL: &str = "KFRC_OPT_LEVEL";
pub const ENV_OPTIMIZATION_GOAL: &str = "KFRC_OPTIMIZATION_GOAL";
pub const ENV_PARALLELISM: &str = "KFRC_PARALLELISM";
pub const ENV_TARGET_CPU: &str = "KFRC_TARGET_CPU";

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Backend {
    #[default]
    Llvm,
    Native,
}

impl Backend {
    pub const fn uses_llvm(self) -> bool {
        matches!(self, Self::Llvm)
    }

    pub const fn performs_ilp_scheduling(self) -> bool {
        match self {
            Self::Llvm => true,
            Self::Native => false,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Llvm => "llvm",
            Self::Native => "native",
        })
    }
}

impl FromStr for Backend {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "llvm" => Ok(Self::Llvm),
            "native" => Ok(Self::Native),
            _ => Err(ConfigError::new(format!(
                "invalid backend {value:?}; expected `llvm` or `native`"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OptLevel {
    O0,
    O1,
    #[default]
    O2,
    O3,
    Os,
    Oz,
}

impl OptLevel {
    pub const fn as_rustc_str(self) -> &'static str {
        match self {
            Self::O0 => "0",
            Self::O1 => "1",
            Self::O2 => "2",
            Self::O3 => "3",
            Self::Os => "s",
            Self::Oz => "z",
        }
    }
}

impl fmt::Display for OptLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_rustc_str())
    }
}

impl FromStr for OptLevel {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "0" | "o0" => Ok(Self::O0),
            "1" | "o1" => Ok(Self::O1),
            "2" | "o2" => Ok(Self::O2),
            "3" | "o3" => Ok(Self::O3),
            "s" | "os" => Ok(Self::Os),
            "z" | "oz" => Ok(Self::Oz),
            _ => Err(ConfigError::new(format!(
                "invalid optimization level {value:?}; expected 0, 1, 2, 3, s, or z"
            ))),
        }
    }
}

pub type OptimizationLevel = OptLevel;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OptimizationGoal {
    Size,
    Speed,
    #[default]
    Balanced,
}

impl fmt::Display for OptimizationGoal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Size => "size",
            Self::Speed => "speed",
            Self::Balanced => "balanced",
        })
    }
}

impl FromStr for OptimizationGoal {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "size" => Ok(Self::Size),
            "speed" => Ok(Self::Speed),
            "balanced" => Ok(Self::Balanced),
            _ => Err(ConfigError::new(format!(
                "invalid optimization goal {value:?}; expected `size`, `speed`, or `balanced`"
            ))),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum TargetCpu {
    #[default]
    Portable,
    Native,
    Named(String),
}

impl TargetCpu {
    pub fn named(name: impl Into<String>) -> Result<Self, ConfigError> {
        let target = Self::Named(name.into());
        target.validate()?;
        Ok(target)
    }

    pub const fn is_portable(&self) -> bool {
        matches!(self, Self::Portable)
    }

    pub const fn is_host_specific(&self) -> bool {
        matches!(self, Self::Native)
    }

    pub fn as_rustc_str(&self) -> &str {
        match self {
            Self::Portable => "generic",
            Self::Native => "native",
            Self::Named(name) => name,
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Self::Named(name) = self {
            let valid = !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
            if !valid {
                return Err(ConfigError::new(format!(
                    "invalid target CPU {name:?}; expected a non-empty CPU name containing only ASCII letters, digits, `-`, `_`, `.`, or `+`"
                )));
            }
        }
        Ok(())
    }
}

impl fmt::Display for TargetCpu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Portable => formatter.write_str("portable"),
            Self::Native => formatter.write_str("native"),
            Self::Named(name) => formatter.write_str(name),
        }
    }
}

impl FromStr for TargetCpu {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "portable" | "generic" => Ok(Self::Portable),
            "native" => Ok(Self::Native),
            _ => Self::named(value),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub backend: Backend,
    pub opt_level: OptLevel,
    pub optimization_goal: OptimizationGoal,
    pub parallelism: usize,
    pub target_cpu: TargetCpu,
}

impl Config {
    pub fn native() -> Self {
        Self {
            backend: Backend::Native,
            ..Self::default()
        }
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();
        for (name, apply) in [
            (
                ENV_BACKEND,
                apply_backend as fn(&mut Self, &str) -> Result<(), ConfigError>,
            ),
            (ENV_OPT_LEVEL, apply_opt_level),
            (ENV_OPTIMIZATION_GOAL, apply_optimization_goal),
            (ENV_PARALLELISM, apply_parallelism),
            (ENV_TARGET_CPU, apply_target_cpu),
        ] {
            match env::var(name) {
                Ok(value) => apply(&mut config, &value)
                    .map_err(|error| error.with_context(format!("in {name}")))?,
                Err(env::VarError::NotPresent) => {}
                Err(env::VarError::NotUnicode(_)) => {
                    return Err(ConfigError::new(format!(
                        "environment variable {name} is not valid Unicode"
                    )));
                }
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn from_lookup<F>(mut lookup: F) -> Result<Self, ConfigError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut config = Self::default();
        for (name, apply) in [
            (
                ENV_BACKEND,
                apply_backend as fn(&mut Self, &str) -> Result<(), ConfigError>,
            ),
            (ENV_OPT_LEVEL, apply_opt_level),
            (ENV_OPTIMIZATION_GOAL, apply_optimization_goal),
            (ENV_PARALLELISM, apply_parallelism),
            (ENV_TARGET_CPU, apply_target_cpu),
        ] {
            if let Some(value) = lookup(name) {
                apply(&mut config, &value)
                    .map_err(|error| error.with_context(format!("in {name}")))?;
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.parallelism == 0 {
            return Err(ConfigError::new("parallelism must be at least 1"));
        }
        self.target_cpu.validate()
    }

    pub const fn uses_llvm(&self) -> bool {
        self.backend.uses_llvm()
    }

    pub const fn performs_ilp_scheduling(&self) -> bool {
        self.backend.performs_ilp_scheduling()
    }

    pub const fn jobs(&self) -> usize {
        self.parallelism
    }
}

impl Default for Config {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1);
        Self {
            backend: Backend::Llvm,
            opt_level: OptLevel::O2,
            optimization_goal: OptimizationGoal::Balanced,
            parallelism,
            target_cpu: TargetCpu::Portable,
        }
    }
}

fn apply_backend(config: &mut Config, value: &str) -> Result<(), ConfigError> {
    config.backend = value.parse()?;
    Ok(())
}

fn apply_opt_level(config: &mut Config, value: &str) -> Result<(), ConfigError> {
    config.opt_level = value.parse()?;
    Ok(())
}

fn apply_optimization_goal(config: &mut Config, value: &str) -> Result<(), ConfigError> {
    config.optimization_goal = value.parse()?;
    Ok(())
}

fn apply_parallelism(config: &mut Config, value: &str) -> Result<(), ConfigError> {
    config.parallelism = value.trim().parse().map_err(|_| {
        ConfigError::new(format!(
            "invalid parallelism {value:?}; expected an integer of at least 1"
        ))
    })?;
    if config.parallelism == 0 {
        return Err(ConfigError::new("parallelism must be at least 1"));
    }
    Ok(())
}

fn apply_target_cpu(config: &mut Config, value: &str) -> Result<(), ConfigError> {
    config.target_cpu = value.parse()?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn with_context(self, context: impl AsRef<str>) -> Self {
        Self::new(format!("{} {}", context.as_ref(), self.message))
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_are_optimized_portable_and_bounded() {
        let config = Config::default();
        assert_eq!(config.backend, Backend::Llvm);
        assert_eq!(config.opt_level, OptLevel::O2);
        assert_eq!(config.optimization_goal, OptimizationGoal::Balanced);
        assert!(config.parallelism >= 1);
        assert_eq!(config.target_cpu, TargetCpu::Portable);
        assert!(config.uses_llvm());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn native_default_has_no_llvm_or_ilp_scheduling() {
        let config = Config::native();
        assert_eq!(config.backend, Backend::Native);
        assert!(!config.uses_llvm());
        assert!(!config.performs_ilp_scheduling());
        assert_eq!(config.target_cpu, TargetCpu::Portable);
        assert_ne!(config.opt_level, OptLevel::O0);
    }

    #[test]
    fn backend_parsing_is_case_insensitive_and_rejects_other_backends() {
        assert_eq!(" LLVM ".parse(), Ok(Backend::Llvm));
        assert_eq!("Native".parse(), Ok(Backend::Native));
        assert!("cranelift".parse::<Backend>().is_err());
        assert!("".parse::<Backend>().is_err());
    }

    #[test]
    fn optimization_levels_parse_rustc_spellings() {
        for (text, level) in [
            ("0", OptLevel::O0),
            ("o1", OptLevel::O1),
            ("2", OptLevel::O2),
            ("O3", OptLevel::O3),
            ("s", OptLevel::Os),
            ("oz", OptLevel::Oz),
        ] {
            assert_eq!(text.parse(), Ok(level));
            assert_eq!(level.as_rustc_str(), level.to_string());
        }
        assert!("fast".parse::<OptLevel>().is_err());
    }

    #[test]
    fn optimization_goals_round_trip() {
        for (text, goal) in [
            ("size", OptimizationGoal::Size),
            ("speed", OptimizationGoal::Speed),
            ("balanced", OptimizationGoal::Balanced),
        ] {
            assert_eq!(text.parse(), Ok(goal));
            assert_eq!(goal.to_string().parse(), Ok(goal));
        }
        assert!("runtime".parse::<OptimizationGoal>().is_err());
    }

    #[test]
    fn target_cpu_distinguishes_portable_native_and_named() {
        let portable = "portable".parse::<TargetCpu>().unwrap();
        assert!(portable.is_portable());
        assert!(!portable.is_host_specific());
        assert_eq!(portable.as_rustc_str(), "generic");

        let native = "native".parse::<TargetCpu>().unwrap();
        assert!(!native.is_portable());
        assert!(native.is_host_specific());
        assert_eq!(native.as_rustc_str(), "native");

        let named = "x86-64-v3".parse::<TargetCpu>().unwrap();
        assert_eq!(named, TargetCpu::Named("x86-64-v3".into()));
        assert_eq!(named.to_string(), "x86-64-v3");
        assert!("bad cpu".parse::<TargetCpu>().is_err());
        assert!("-native".parse::<TargetCpu>().is_err());
        assert!("".parse::<TargetCpu>().is_err());
    }

    #[test]
    fn environment_lookup_overrides_every_setting() {
        let values = HashMap::from([
            (ENV_BACKEND, "native"),
            (ENV_OPT_LEVEL, "z"),
            (ENV_OPTIMIZATION_GOAL, "size"),
            (ENV_PARALLELISM, "3"),
            (ENV_TARGET_CPU, "native"),
        ]);
        let config =
            Config::from_lookup(|name| values.get(name).map(|value| (*value).into())).unwrap();
        assert_eq!(config.backend, Backend::Native);
        assert_eq!(config.opt_level, OptLevel::Oz);
        assert_eq!(config.optimization_goal, OptimizationGoal::Size);
        assert_eq!(config.parallelism, 3);
        assert_eq!(config.jobs(), 3);
        assert_eq!(config.target_cpu, TargetCpu::Native);
        assert!(!config.uses_llvm());
        assert!(!config.performs_ilp_scheduling());
    }

    #[test]
    fn absent_environment_values_keep_defaults() {
        let defaults = Config::default();
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config, defaults);
    }

    #[test]
    fn invalid_environment_values_identify_the_variable() {
        let error = Config::from_lookup(|name| (name == ENV_PARALLELISM).then(|| "0".to_string()))
            .unwrap_err();
        assert!(error.to_string().contains(ENV_PARALLELISM));
        assert!(error.to_string().contains("at least 1"));

        let error = Config::from_lookup(|name| (name == ENV_BACKEND).then(|| "other".to_string()))
            .unwrap_err();
        assert!(error.to_string().contains(ENV_BACKEND));
    }

    #[test]
    fn validation_rejects_zero_parallelism_and_bad_direct_target_names() {
        let config = Config {
            parallelism: 0,
            ..Config::default()
        };
        assert!(config.validate().is_err());

        let config = Config {
            target_cpu: TargetCpu::Named("cpu,experimental".into()),
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }
}
