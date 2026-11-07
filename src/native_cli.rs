use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::string::FromUtf8Error;

use crate::frontend::{self, ParseError};
use crate::native::{self, CompileError};
use crate::native_cache::{self, CachedObject, NativeCache, NativeCacheError};
use crate::native_driver::{
    NativeDriver, NativeDriverError, NativeOutputKind, NativeOutputOptions,
};
use crate::native_object::{self, NativeObjectError};
use crate::optimizer_pipeline;
use crate::placement::{self, PlacementError};
use crate::son::Graph;

const USAGE: &str = "usage: kfrc native <input> -o <output> [--emit=asm | --emit=obj | -S]\n       kfrc native <input> [--emit=asm=<path> | --emit=obj=<path>]";
const NATIVE_OBJECT_BACKEND_VERSION: &str = "native-x86_64-elf-v1";

#[derive(Debug)]
pub enum NativeCliError {
    Arguments(String),
    Read {
        path: PathBuf,
        source: io::Error,
    },
    InvalidUtf8 {
        path: PathBuf,
        source: FromUtf8Error,
    },
    Parse {
        path: PathBuf,
        source: ParseError,
    },
    Placement(PlacementError),
    Compile(CompileError),
    Object(NativeObjectError),
    Cache(NativeCacheError),
    Driver(NativeDriverError),
}

impl fmt::Display for NativeCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arguments(message) => write!(formatter, "{message}\n{USAGE}"),
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "failed to read input {}: {source}",
                    path.display()
                )
            }
            Self::InvalidUtf8 { path, source } => write!(
                formatter,
                "input {} is not valid UTF-8: {source}",
                path.display()
            ),
            Self::Parse { path, source } => {
                write!(formatter, "{}:{source}", path.display())
            }
            Self::Placement(source) => write!(formatter, "native graph placement failed: {source}"),
            Self::Compile(source) => write!(formatter, "native compilation failed: {source}"),
            Self::Object(source) => source.fmt(formatter),
            Self::Cache(source) => source.fmt(formatter),
            Self::Driver(source) => source.fmt(formatter),
        }
    }
}

impl Error for NativeCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Arguments(_) => None,
            Self::Read { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Placement(source) => Some(source),
            Self::Compile(source) => Some(source),
            Self::Object(source) => Some(source),
            Self::Cache(source) => Some(source),
            Self::Driver(source) => Some(source),
        }
    }
}

impl From<PlacementError> for NativeCliError {
    fn from(source: PlacementError) -> Self {
        Self::Placement(source)
    }
}

impl From<CompileError> for NativeCliError {
    fn from(source: CompileError) -> Self {
        Self::Compile(source)
    }
}

impl From<NativeObjectError> for NativeCliError {
    fn from(source: NativeObjectError) -> Self {
        Self::Object(source)
    }
}

impl From<NativeCacheError> for NativeCliError {
    fn from(source: NativeCacheError) -> Self {
        Self::Cache(source)
    }
}

impl From<NativeDriverError> for NativeCliError {
    fn from(source: NativeDriverError) -> Self {
        Self::Driver(source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeEmitKind {
    Assembly,
    Executable,
    Object,
}

#[derive(Debug)]
struct NativeCliOptions {
    input: PathBuf,
    output: PathBuf,
    emit: NativeEmitKind,
}

pub fn run_with_args<I, S>(args: I) -> Result<PathBuf, NativeCliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let options = parse_args(args)?;
    run_with_driver(&options, &NativeDriver::new())
}

fn run_with_driver(
    options: &NativeCliOptions,
    driver: &NativeDriver,
) -> Result<PathBuf, NativeCliError> {
    let cache_directory = native_cache_directory();
    run_with_driver_and_cache(options, driver, cache_directory.as_deref())
}

fn native_cache_directory() -> Option<PathBuf> {
    let path = std::env::var_os("KFRC_NATIVE_CACHE")?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn run_with_driver_and_cache(
    options: &NativeCliOptions,
    driver: &NativeDriver,
    cache_directory: Option<&std::path::Path>,
) -> Result<PathBuf, NativeCliError> {
    let bytes = fs::read(&options.input).map_err(|source| NativeCliError::Read {
        path: options.input.clone(),
        source,
    })?;
    let source = String::from_utf8(bytes).map_err(|source| NativeCliError::InvalidUtf8 {
        path: options.input.clone(),
        source,
    })?;
    let mut graph = frontend::parse(&source).map_err(|source| NativeCliError::Parse {
        path: options.input.clone(),
        source,
    })?;
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_graph(&graph)?;
    if options.emit == NativeEmitKind::Object {
        let object = compile_object(&graph, cache_directory)?;
        fs::write(&options.output, object.bytes).map_err(|source| NativeObjectError::Write {
            path: options.output.clone(),
            source,
        })?;
        return Ok(options.output.clone());
    }

    let assembly = native::compile_graph_to_assembly(&graph, "main")?;
    let kind = if options.emit == NativeEmitKind::Assembly {
        NativeOutputKind::Assembly
    } else {
        NativeOutputKind::Executable
    };
    let output = NativeOutputOptions {
        output: options.output.clone(),
        kind,
    };
    Ok(driver.compile(&assembly, &output)?)
}

fn compile_object(
    graph: &Graph,
    cache_directory: Option<&std::path::Path>,
) -> Result<CachedObject, NativeCliError> {
    if let Some(directory) = cache_directory {
        return Ok(NativeCache::new(directory).get_or_compile(
            graph,
            "main",
            native_cache::SUPPORTED_TARGET,
            native_cache::SUPPORTED_ABI,
            NATIVE_OBJECT_BACKEND_VERSION,
        )?);
    }
    Ok(CachedObject {
        bytes: native_object::compile_graph_to_object(graph, "main")?,
        cache_hit: false,
    })
}

fn parse_args<I, S>(args: I) -> Result<NativeCliOptions, NativeCliError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut input = None;
    let mut output = None;
    let mut emit = None;

    while let Some(argument) = args.next() {
        if argument == "-o" {
            let path = args
                .next()
                .ok_or_else(|| argument_error("missing path after -o; expected -o <output>"))?;
            if path.is_empty() {
                return Err(argument_error("output path after -o must not be empty"));
            }
            set_output(&mut output, PathBuf::from(path))?;
        } else if argument == "-S" {
            set_emit(&mut emit, NativeEmitKind::Assembly)?;
        } else if let Some(path) = emit_path(&argument, "--emit=asm") {
            set_emit(&mut emit, NativeEmitKind::Assembly)?;
            if let Some(path) = path {
                if path.as_os_str().is_empty() {
                    return Err(argument_error(
                        "missing path after --emit=asm=; expected --emit=asm=<path>",
                    ));
                }
                set_output(&mut output, path)?;
            }
        } else if let Some(path) = emit_path(&argument, "--emit=obj") {
            set_emit(&mut emit, NativeEmitKind::Object)?;
            if let Some(path) = path {
                if path.as_os_str().is_empty() {
                    return Err(argument_error(
                        "missing path after --emit=obj=; expected --emit=obj=<path>",
                    ));
                }
                set_output(&mut output, path)?;
            }
        } else if starts_with_ascii(&argument, "--emit") {
            return Err(argument_error(
                "invalid --emit option; expected --emit=asm, --emit=asm=<path>, --emit=obj, or --emit=obj=<path>",
            ));
        } else if starts_with_ascii(&argument, "-") {
            return Err(argument_error(format!(
                "unknown native option {argument:?}"
            )));
        } else if input.is_some() {
            return Err(argument_error(
                "multiple input files were provided; expected exactly one input",
            ));
        } else {
            if argument.is_empty() {
                return Err(argument_error("input path must not be empty"));
            }
            input = Some(PathBuf::from(argument));
        }
    }

    let input = input.ok_or_else(|| argument_error("missing input path"))?;
    let output = output.ok_or_else(|| {
        argument_error(
            "missing output path; use -o <output>, --emit=asm=<path>, or --emit=obj=<path>",
        )
    })?;

    Ok(NativeCliOptions {
        input,
        output,
        emit: emit.unwrap_or(NativeEmitKind::Executable),
    })
}

fn set_output(output: &mut Option<PathBuf>, path: PathBuf) -> Result<(), NativeCliError> {
    if output.is_some() {
        return Err(argument_error(
            "output path was provided more than once; use only one of -o, --emit=asm=<path>, and --emit=obj=<path>",
        ));
    }
    *output = Some(path);
    Ok(())
}

fn set_emit(emit: &mut Option<NativeEmitKind>, kind: NativeEmitKind) -> Result<(), NativeCliError> {
    if let Some(previous) = emit {
        if *previous == NativeEmitKind::Assembly && kind == NativeEmitKind::Assembly {
            return Err(argument_error(
                "assembly output was requested more than once; use only one of -S and --emit=asm",
            ));
        }
        return Err(argument_error(
            "output format was requested more than once; use only one of -S, --emit=asm, and --emit=obj",
        ));
    }
    *emit = Some(kind);
    Ok(())
}

fn argument_error(message: impl Into<String>) -> NativeCliError {
    NativeCliError::Arguments(message.into())
}

fn starts_with_ascii(argument: &OsStr, prefix: &str) -> bool {
    argument.as_encoded_bytes().starts_with(prefix.as_bytes())
}

#[cfg(unix)]
fn emit_path(argument: &OsStr, prefix: &str) -> Option<Option<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;

    let suffix = argument.as_bytes().strip_prefix(prefix.as_bytes())?;
    match suffix {
        [] => Some(None),
        [b'=', path @ ..] => Some(Some(PathBuf::from(OsStr::from_bytes(path)))),
        _ => None,
    }
}

#[cfg(not(unix))]
fn emit_path(argument: &OsStr, prefix: &str) -> Option<Option<PathBuf>> {
    let suffix = argument.to_str()?.strip_prefix(prefix)?;
    match suffix.strip_prefix('=') {
        Some(path) => Some(Some(PathBuf::from(path))),
        None if suffix.is_empty() => Some(None),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = env_temp_dir().join(format!(
                "kfrc-native-cli-test-{}-{timestamp:x}-{id:x}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn env_temp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    fn strings(arguments: &[&str]) -> Vec<OsString> {
        arguments.iter().map(OsString::from).collect()
    }

    fn arguments_error(arguments: Vec<OsString>) -> String {
        match parse_args(arguments) {
            Ok(_) => panic!("arguments unexpectedly succeeded"),
            Err(error) => error.to_string(),
        }
    }

    fn object_options(input: &Path, output: &Path) -> NativeCliOptions {
        parse_args(vec![
            input.as_os_str().to_owned(),
            OsString::from("--emit=obj"),
            OsString::from("-o"),
            output.as_os_str().to_owned(),
        ])
        .unwrap()
    }

    fn cache_entries(directory: &Path) -> Vec<PathBuf> {
        let mut entries = Vec::new();
        for prefix in fs::read_dir(directory).unwrap() {
            for entry in fs::read_dir(prefix.unwrap().path()).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "o") {
                    entries.push(path);
                }
            }
        }
        entries.sort();
        entries
    }

    #[test]
    fn parses_executable_assembly_and_object_output_forms() {
        let executable = parse_args(strings(&["input.rs", "-o", "program"])).unwrap();
        assert_eq!(executable.input, Path::new("input.rs"));
        assert_eq!(executable.output, Path::new("program"));
        assert_eq!(executable.emit, NativeEmitKind::Executable);

        for arguments in [
            strings(&["input.rs", "-o", "program.s", "--emit=asm"]),
            strings(&["-S", "input.rs", "-o", "program.s"]),
        ] {
            let options = parse_args(arguments).unwrap();
            assert_eq!(options.output, Path::new("program.s"));
            assert_eq!(options.emit, NativeEmitKind::Assembly);
        }

        let inline = parse_args(strings(&["input.rs", "--emit=asm=program.s"])).unwrap();
        assert_eq!(inline.output, Path::new("program.s"));
        assert_eq!(inline.emit, NativeEmitKind::Assembly);

        for arguments in [
            strings(&["input.rs", "--emit=obj", "-o", "program.o"]),
            strings(&["input.rs", "--emit=obj=program.o"]),
        ] {
            let options = parse_args(arguments).unwrap();
            assert_eq!(options.output, Path::new("program.o"));
            assert_eq!(options.emit, NativeEmitKind::Object);
        }
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_arguments() {
        let cases = [
            (strings(&[]), "missing input path"),
            (strings(&["input.rs"]), "missing output path"),
            (strings(&["input.rs", "-o"]), "missing path after -o"),
            (
                strings(&["input.rs", "-o", "a", "-o", "b"]),
                "output path was provided more than once",
            ),
            (
                strings(&["input.rs", "--emit=asm=a", "-o", "b"]),
                "output path was provided more than once",
            ),
            (
                strings(&["input.rs", "-o", "a", "-S", "--emit=asm"]),
                "assembly output was requested more than once",
            ),
            (
                strings(&["input.rs", "other.rs", "-o", "a"]),
                "multiple input files",
            ),
            (
                strings(&["input.rs", "-o", "a", "--unknown"]),
                "unknown native option \"--unknown\"",
            ),
            (
                strings(&["input.rs", "--emit=llvm", "-o", "a"]),
                "invalid --emit option",
            ),
            (
                strings(&["input.rs", "--emit=asm="]),
                "missing path after --emit=asm=",
            ),
            (strings(&["input.rs", "--emit=obj"]), "missing output path"),
            (
                strings(&["input.rs", "--emit=obj="]),
                "missing path after --emit=obj=",
            ),
            (
                strings(&["input.rs", "--emit=obj=a", "-o", "b"]),
                "output path was provided more than once",
            ),
            (
                strings(&["input.rs", "--emit=obj", "--emit=asm", "-o", "a"]),
                "output format was requested more than once",
            ),
            (
                strings(&["input.rs", "--emit=obj", "--emit=obj", "-o", "a"]),
                "output format was requested more than once",
            ),
        ];

        for (arguments, expected) in cases {
            let error = arguments_error(arguments);
            assert!(error.contains(expected), "unexpected error: {error}");
            assert!(error.contains("usage: kfrc native"));
        }
    }

    #[test]
    fn compiles_optimized_source_to_assembly_without_running_native_tools() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        let output = directory.path.join("output.s");
        fs::write(
            &input,
            "fn main() -> i64 { let x = 6 * 7; return x + 2; }\n",
        )
        .unwrap();
        let driver = NativeDriver::new().with_tools("missing-as", "missing-gcc");
        let options = parse_args(vec![
            input.clone().into_os_string(),
            OsString::from("--emit=asm"),
            OsString::from("-o"),
            output.clone().into_os_string(),
        ])
        .unwrap();

        let returned = run_with_driver(&options, &driver).unwrap();

        assert_eq!(returned, output);
        let assembly = fs::read_to_string(output).unwrap();
        assert!(assembly.starts_with(".intel_syntax noprefix\n.text\n.globl main\n"));
        assert!(assembly.contains("\tmovabs rcx, 0x2c\n"), "{assembly}");
        assert!(!assembly.contains("\timul"), "{assembly}");
        assert!(!assembly.contains("\tadd"), "{assembly}");
        assert!(assembly.contains("\tret"), "{assembly}");
        assert!(assembly.ends_with(".section .note.GNU-stack,\"\",@progbits\n"));
    }

    #[test]
    fn writes_optimized_elf_objects_without_running_native_tools() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        fs::write(
            &input,
            "fn main() -> i64 { let x = 6 * 7; return x + 2; }\n",
        )
        .unwrap();
        let driver = NativeDriver::new().with_tools("missing-as", "missing-gcc");

        for inline_path in [false, true] {
            let output = directory.path.join(if inline_path {
                "inline.o"
            } else {
                "separate.o"
            });
            let mut arguments = vec![input.clone().into_os_string()];
            if inline_path {
                arguments.push(OsString::from(format!("--emit=obj={}", output.display())));
            } else {
                arguments.extend([
                    OsString::from("--emit=obj"),
                    OsString::from("-o"),
                    output.clone().into_os_string(),
                ]);
            }
            let options = parse_args(arguments).unwrap();

            let returned = run_with_driver(&options, &driver).unwrap();

            assert_eq!(returned, output);
            let object = fs::read(&output).unwrap();
            assert_eq!(&object[..4], b"\x7fELF");
            assert_eq!(&object[16..18], &[1, 0], "object must be relocatable");
            assert_eq!(&object[18..20], &[62, 0], "object must target x86_64");
            assert!(object.windows(6).any(|bytes| bytes == b"\0main\0"));
            assert!(
                object
                    .windows(4)
                    .any(|bytes| bytes == [0x48, 0xb9, 0x2c, 0]),
                "optimized object should contain movabs rcx, 44"
            );
        }
    }

    #[test]
    fn object_cache_hit_reuses_the_cached_entry() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        let output = directory.path.join("output.o");
        let cache = directory.path.join("cache");
        fs::write(&input, "fn main() { return 42; }").unwrap();
        let options = object_options(&input, &output);
        let driver = NativeDriver::new().with_tools("missing-as", "missing-gcc");

        run_with_driver_and_cache(&options, &driver, Some(&cache)).unwrap();
        let expected = fs::read(&output).unwrap();
        let entries = cache_entries(&cache);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        fs::File::open(entry)
            .unwrap()
            .set_modified(UNIX_EPOCH)
            .unwrap();

        let returned = run_with_driver_and_cache(&options, &driver, Some(&cache)).unwrap();

        assert_eq!(returned, output);
        assert_eq!(fs::read(output).unwrap(), expected);
        assert_eq!(cache_entries(&cache), entries);
        assert_eq!(
            fs::metadata(entry).unwrap().modified().unwrap(),
            UNIX_EPOCH,
            "a cache hit must not recompile and replace the entry"
        );
    }

    #[test]
    fn object_cache_recompiles_when_the_placed_graph_changes() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        let output = directory.path.join("output.o");
        let cache = directory.path.join("cache");
        let options = object_options(&input, &output);
        let driver = NativeDriver::new().with_tools("missing-as", "missing-gcc");

        fs::write(&input, "fn main() { return 42; }").unwrap();
        run_with_driver_and_cache(&options, &driver, Some(&cache)).unwrap();
        let first = fs::read(&output).unwrap();
        assert_eq!(cache_entries(&cache).len(), 1);

        fs::write(&input, "fn main() { return 43; }").unwrap();
        run_with_driver_and_cache(&options, &driver, Some(&cache)).unwrap();
        let second = fs::read(&output).unwrap();
        assert_ne!(second, first);
        assert!(
            second
                .windows(4)
                .any(|bytes| bytes == [0x48, 0xb9, 0x2b, 0])
        );
        assert_eq!(cache_entries(&cache).len(), 2);

        fs::write(&input, "fn main() { return 42; }").unwrap();
        run_with_driver_and_cache(&options, &driver, Some(&cache)).unwrap();
        assert_eq!(fs::read(output).unwrap(), first);
        assert_eq!(cache_entries(&cache).len(), 2);
    }

    #[test]
    fn object_write_errors_include_the_output_path() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        let output = directory.path.join("missing").join("output.o");
        fs::write(&input, "fn main() { return 42; }").unwrap();
        let options = parse_args(vec![
            input.into_os_string(),
            OsString::from("--emit=obj"),
            OsString::from("-o"),
            output.clone().into_os_string(),
        ])
        .unwrap();

        let error = run_with_driver(&options, &NativeDriver::new()).unwrap_err();

        assert!(matches!(
            error,
            NativeCliError::Object(NativeObjectError::Write { .. })
        ));
        let message = error.to_string();
        assert!(message.contains("failed to write object"), "{message}");
        assert!(message.contains(&output.display().to_string()), "{message}");
    }

    #[test]
    fn placement_errors_have_explicit_display_and_source() {
        let source = PlacementError::MissingReturn;
        let error = NativeCliError::from(source.clone());

        assert!(matches!(error, NativeCliError::Placement(_)));
        assert_eq!(
            error.to_string(),
            "native graph placement failed: graph has no return node"
        );
        assert_eq!(
            std::error::Error::source(&error).unwrap().to_string(),
            source.to_string()
        );
    }

    #[test]
    fn reports_invalid_utf8_and_parse_errors_with_input_path() {
        let directory = TestDirectory::new();
        let input = directory.path.join("input.rs");
        let output = directory.path.join("output.s");
        fs::write(&input, [0xff, 0xfe]).unwrap();
        let options = parse_args(vec![
            input.clone().into_os_string(),
            OsString::from("-S"),
            OsString::from("-o"),
            output.clone().into_os_string(),
        ])
        .unwrap();

        let error = run_with_driver(&options, &NativeDriver::new()).unwrap_err();
        assert!(matches!(error, NativeCliError::InvalidUtf8 { .. }));
        assert!(error.to_string().contains("not valid UTF-8"));

        fs::write(&input, "fn main() { return missing; }").unwrap();
        let error = run_with_driver(&options, &NativeDriver::new()).unwrap_err();
        assert!(matches!(error, NativeCliError::Parse { .. }));
        let message = error.to_string();
        assert!(message.contains("input.rs:1:20"), "{message}");
        assert!(message.contains("unknown variable `missing`"));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_input_and_inline_output_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let input = OsString::from_vec(b"input-\xff.rs".to_vec());
        for (prefix, output, kind) in [
            (
                "--emit=asm=",
                b"output-\xfe.s".to_vec(),
                NativeEmitKind::Assembly,
            ),
            (
                "--emit=obj=",
                b"output-\xfe.o".to_vec(),
                NativeEmitKind::Object,
            ),
        ] {
            let output = PathBuf::from(OsString::from_vec(output));
            let mut emit = prefix.as_bytes().to_vec();
            emit.extend_from_slice(output.as_os_str().as_bytes());

            let options = parse_args(vec![input.clone(), OsString::from_vec(emit)]).unwrap();

            assert_eq!(options.input, PathBuf::from(input.clone()));
            assert_eq!(options.output, output);
            assert_eq!(options.emit, kind);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn supports_non_utf8_input_and_inline_assembly_output_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let directory = TestDirectory::new();
        let input = directory
            .path
            .join(OsString::from_vec(b"input-\xff.rs".to_vec()));
        let output = directory
            .path
            .join(OsString::from_vec(b"output-\xfe.s".to_vec()));
        fs::write(&input, "fn main() { return 42; }").unwrap();
        let mut emit = b"--emit=asm=".to_vec();
        emit.extend_from_slice(output.as_os_str().as_bytes());

        let returned =
            run_with_args(vec![input.into_os_string(), OsString::from_vec(emit)]).unwrap();

        assert_eq!(returned, output);
        let assembly = fs::read_to_string(output).unwrap();
        assert!(assembly.contains(".globl main"));
        assert!(assembly.contains("movabs rcx, 0x2a"));
    }
}
